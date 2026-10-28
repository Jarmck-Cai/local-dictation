use super::super::queue::capture_event_queue;
use super::*;

#[test]
fn cancellation_during_stop_clock_sampling_cannot_certify_a_healthy_end() {
    let stop = AtomicBool::new(false);
    let boundary = std::sync::Mutex::new(());
    let end = request_stop_boundary(&stop, &boundary, || {
        stop.store(true, Ordering::Release);
        Ok(200)
    })
    .unwrap();
    assert_eq!(end, None);
}

#[test]
fn stop_between_health_check_and_clock_publication_cannot_publish_beyond_end() {
    let (sender, receiver) = capture_event_queue();
    let stop = AtomicBool::new(false);
    let boundary = std::sync::Mutex::new(());
    let end = request_stop_boundary(&stop, &boundary, || Ok(200)).unwrap();
    let heartbeat = publish_heartbeat(&sender, &stop, &boundary, Source::Microphone, 100, || {
        panic!("a stopped worker must not sample a new heartbeat timestamp")
    })
    .unwrap();
    assert_eq!(heartbeat, None);
    complete_interval(100, end, Ok(()), &sender, &stop).unwrap();
    for source in [Source::Microphone, Source::Remote] {
        assert!(
            matches!(receiver.try_recv().unwrap(), CaptureEvent::Heartbeat {
            source: actual, qpc_100ns: 200
        } if actual == source)
        );
    }
    assert!(receiver.try_recv().is_err());
}

#[test]
fn a_published_health_clock_precedes_the_serialized_stop_boundary() {
    let (sender, receiver) = capture_event_queue();
    let stop = AtomicBool::new(false);
    let boundary = std::sync::Mutex::new(());
    assert_eq!(
        publish_heartbeat(&sender, &stop, &boundary, Source::Microphone, 100, || Ok(
            200
        ))
        .unwrap(),
        Some(200)
    );
    let end = request_stop_boundary(&stop, &boundary, || Ok(300)).unwrap();
    complete_interval(100, end, Ok(()), &sender, &stop).unwrap();
    assert!(matches!(
        receiver.try_recv().unwrap(),
        CaptureEvent::Heartbeat { qpc_100ns: 200, .. }
    ));
    for _ in 0..2 {
        assert!(matches!(
            receiver.try_recv().unwrap(),
            CaptureEvent::Heartbeat { qpc_100ns: 300, .. }
        ));
    }
}

fn pcm16() -> AudioFormat {
    AudioFormat {
        sample_rate: 48_000,
        channels: 2,
        bits_per_sample: 16,
        valid_bits: 16,
        channel_mask: 0,
        encoding: AudioEncoding::Pcm,
    }
}

fn wave(tag: u16, channels: u16, bits: u16, valid: u16, subtag: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; if tag == 0xfffe { 40 } else { 18 }];
    bytes[0..2].copy_from_slice(&tag.to_le_bytes());
    bytes[2..4].copy_from_slice(&channels.to_le_bytes());
    bytes[4..8].copy_from_slice(&48_000u32.to_le_bytes());
    let align = channels * (bits / 8);
    bytes[8..12].copy_from_slice(&(48_000u32 * u32::from(align)).to_le_bytes());
    bytes[12..14].copy_from_slice(&align.to_le_bytes());
    bytes[14..16].copy_from_slice(&bits.to_le_bytes());
    if tag == 0xfffe {
        bytes[16..18].copy_from_slice(&22u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&valid.to_le_bytes());
        bytes[20..24].copy_from_slice(&3u32.to_le_bytes());
        bytes[24..40].copy_from_slice(&[
            subtag, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
        ]);
    }
    bytes
}

#[test]
fn original_pcm_containers_and_extensible_valid_bits_are_retained() {
    for bits in [8, 16, 24, 32] {
        let parsed = parse_wave_format(&wave(1, 2, bits, bits, 1)).unwrap();
        assert_eq!(
            parsed,
            AudioFormat {
                sample_rate: 48_000,
                channels: 2,
                bits_per_sample: bits,
                valid_bits: bits,
                channel_mask: 0,
                encoding: AudioEncoding::Pcm
            }
        );
    }
    let parsed = parse_wave_format(&wave(0xfffe, 2, 32, 24, 1)).unwrap();
    assert_eq!(parsed.bits_per_sample, 32);
    assert_eq!(parsed.valid_bits, 24);
    assert_eq!(parsed.channel_mask, 3);
}

#[test]
fn malformed_block_alignment_and_extensible_guid_are_rejected() {
    let mut bytes = wave(1, 2, 16, 16, 1);
    bytes[12..14].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        parse_wave_format(&bytes).unwrap_err(),
        "meeting_capture_format_invalid"
    );
    let mut bytes = wave(0xfffe, 2, 32, 24, 1);
    bytes[39] = 0;
    assert_eq!(
        parse_wave_format(&bytes).unwrap_err(),
        "meeting_capture_format_unsupported"
    );
    assert!(parse_wave_format(&bytes[..23]).is_err());
    assert!(parse_wave_format(&wave(0xfffe, 2, 16, 24, 1)).is_err());
}

#[test]
fn sample_rates_below_storage_support_fail_before_capture_starts() {
    let mut bytes = wave(1, 2, 16, 16, 1);
    bytes[4..8].copy_from_slice(&4000u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&16000u32.to_le_bytes());
    assert_eq!(
        parse_wave_format(&bytes).unwrap_err(),
        "meeting_capture_format_invalid"
    );
}

#[test]
fn null_silent_packet_keeps_duration_flags_and_qpc_without_reading_data() {
    let packet = unsafe {
        copy_packet(
            Source::Remote,
            4,
            9,
            88,
            123_456_789,
            480,
            7,
            &pcm16(),
            std::ptr::null(),
        )
    }
    .unwrap()
    .unwrap();
    assert_eq!(packet.frames, 480);
    assert!(packet.data.is_empty());
    assert_eq!(packet.flags, 7);
    assert_eq!(packet.qpc_100ns, 123_456_789);
    assert_eq!(packet.device_frame_position, 88);
    assert_eq!(packet.epoch, 4);
    assert_eq!(packet.sequence, 9);
}

#[test]
fn zero_frames_does_not_read_a_stale_non_null_pointer() {
    let packet = unsafe {
        copy_packet(
            Source::Microphone,
            0,
            0,
            0,
            0,
            0,
            0,
            &pcm16(),
            std::ptr::dangling::<u8>(),
        )
    }
    .unwrap();
    assert!(packet.is_none());
}

#[test]
fn non_silent_packet_has_exact_original_bytes_and_null_is_an_error() {
    let bytes = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let packet = unsafe {
        copy_packet(
            Source::Microphone,
            0,
            0,
            10,
            20,
            2,
            1,
            &pcm16(),
            bytes.as_ptr(),
        )
    }
    .unwrap()
    .unwrap();
    assert_eq!(packet.data, bytes);
    assert_eq!(packet.flags, 1);
    let error = unsafe {
        copy_packet(
            Source::Microphone,
            0,
            0,
            0,
            0,
            1,
            0,
            &pcm16(),
            std::ptr::null(),
        )
    }
    .unwrap_err();
    assert_eq!(error, "meeting_capture_null_packet");
}

#[test]
fn oversized_packet_is_rejected_before_touching_pointer() {
    let error = unsafe {
        copy_packet(
            Source::Remote,
            0,
            0,
            0,
            0,
            u32::MAX,
            0,
            &pcm16(),
            std::ptr::dangling::<u8>(),
        )
    }
    .unwrap_err();
    assert_eq!(error, "meeting_capture_packet_too_large");
}

#[test]
fn ieee_float_is_preserved_and_non_finite_samples_are_rejected() {
    let format = parse_wave_format(&wave(0xfffe, 1, 32, 32, 3)).unwrap();
    assert_eq!(format.encoding, AudioEncoding::Float);
    let bytes = [0u8, 0, 0x80, 0x3f, 0, 0, 0, 0xbf];
    let packet = unsafe { copy_packet(Source::Remote, 0, 0, 0, 1, 2, 4, &format, bytes.as_ptr()) }
        .unwrap()
        .unwrap();
    assert_eq!(packet.data, bytes);
    for bytes in [[0u8, 0, 0xc0, 0x7f], [0u8, 0, 0x80, 0x7f]] {
        assert_eq!(
            unsafe { copy_packet(Source::Remote, 0, 0, 0, 1, 1, 0, &format, bytes.as_ptr()) }
                .unwrap_err(),
            "meeting_capture_nonfinite_sample"
        );
    }
}

#[test]
fn bounded_queue_failure_stops_capture_and_does_not_replace_saved_packet() {
    let (sender, receiver) = capture_event_queue();
    let stop = AtomicBool::new(false);
    deliver_event(
        &sender,
        CaptureEvent::Heartbeat {
            source: Source::Microphone,
            qpc_100ns: 7,
        },
        &stop,
    )
    .unwrap();
    for sequence in 1..2048 {
        deliver_event(
            &sender,
            CaptureEvent::Heartbeat {
                source: Source::Microphone,
                qpc_100ns: sequence + 7,
            },
            &stop,
        )
        .unwrap();
    }
    assert_eq!(
        deliver_event(
            &sender,
            CaptureEvent::Heartbeat {
                source: Source::Remote,
                qpc_100ns: 8
            },
            &stop
        )
        .unwrap_err(),
        "meeting_capture_queue_full"
    );
    assert!(stop.load(Ordering::Acquire));
    match receiver.recv().unwrap() {
        CaptureEvent::Heartbeat { qpc_100ns, .. } => assert_eq!(qpc_100ns, 7),
        _ => panic!("first event must remain queued"),
    }
}

#[test]
fn audio_byte_budget_exhaustion_stops_capture_before_event_capacity() {
    let (sender, receiver) = capture_event_queue();
    let stop = AtomicBool::new(false);
    for sequence in 0..4 {
        deliver_event(
            &sender,
            CaptureEvent::Packet(CapturePacket {
                source: Source::Microphone,
                epoch: 0,
                sequence,
                device_frame_position: sequence * 1_048_576,
                qpc_100ns: sequence * 218_453_333,
                frames: 1_048_576,
                flags: 0,
                format: pcm16(),
                data: vec![sequence as u8; 4 * 1024 * 1024],
            }),
            &stop,
        )
        .unwrap();
    }
    let error = deliver_event(
        &sender,
        CaptureEvent::Packet(CapturePacket {
            source: Source::Remote,
            epoch: 0,
            sequence: 4,
            device_frame_position: 0,
            qpc_100ns: 0,
            frames: 1,
            flags: 0,
            format: pcm16(),
            data: vec![0; 4],
        }),
        &stop,
    )
    .unwrap_err();
    assert_eq!(error, "meeting_capture_queue_full");
    assert!(stop.load(Ordering::Acquire));
    for sequence in 0..4 {
        match receiver.recv().unwrap() {
            CaptureEvent::Packet(packet) => {
                assert_eq!(packet.sequence, sequence);
                assert_eq!(packet.data.len(), 4 * 1024 * 1024);
                assert_eq!(packet.data[0], sequence as u8);
            }
            _ => panic!("accepted original packets must stay queued in order"),
        }
    }
    assert!(receiver.try_recv().is_err());
}

#[test]
fn disconnected_writer_stops_both_capture_sources() {
    let (sender, receiver) = capture_event_queue();
    drop(receiver);
    let stop = AtomicBool::new(false);
    assert_eq!(
        deliver_event(
            &sender,
            CaptureEvent::Heartbeat {
                source: Source::Remote,
                qpc_100ns: 0
            },
            &stop
        )
        .unwrap_err(),
        "meeting_capture_consumer_gone"
    );
    assert!(stop.load(Ordering::Acquire));
}

#[test]
fn qpc_ticks_are_converted_once_with_a_wide_intermediate() {
    assert_eq!(
        qpc_to_100ns(9_223_372_036_854_775, 10_000_000).unwrap(),
        9_223_372_036_854_775
    );
    assert_eq!(qpc_to_100ns(12_345_678, 3_000_000).unwrap(), 41_152_260);
    assert!(qpc_to_100ns(-1, 3_000_000).is_err());
    assert!(qpc_to_100ns(1, 0).is_err());
}

#[test]
fn shared_start_is_sampled_only_after_both_engines_started() {
    use std::sync::mpsc::{channel, sync_channel};
    use std::sync::Arc;
    let (notices, receiver) = channel();
    let (events, packets) = capture_event_queue();
    let stop = Arc::new(AtomicBool::new(false));
    let started = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut gates = Vec::new();
    let mut workers = Vec::new();
    for source in [Source::Microphone, Source::Remote] {
        let (gate, commands) = sync_channel(1);
        gates.push(gate);
        let notices = notices.clone();
        let events = events.clone();
        let stop = stop.clone();
        let started = started.clone();
        workers.push(std::thread::spawn(move || {
            notices.send(Notice::Ready).unwrap();
            let Ok(command) = commands.recv() else {
                return;
            };
            match command {
                StartGate::Start => {}
                _ => panic!("neither engine may commit before Start"),
            }
            started.fetch_add(1, Ordering::Release);
            notices.send(Notice::Started).unwrap();
            let Ok(command) = commands.recv() else {
                return;
            };
            let start = match command {
                StartGate::Commit(start) => start,
                _ => panic!("started engine must receive common interval"),
            };
            deliver_event(
                &events,
                CaptureEvent::Heartbeat {
                    source,
                    qpc_100ns: start,
                },
                &stop,
            )
            .unwrap();
            notices.send(Notice::Committed).unwrap();
        }));
    }
    let result = coordinate_startup(&receiver, &gates, &stop, || {
        assert_eq!(started.load(Ordering::Acquire), 2);
        Ok(100_000_000)
    });
    drop(gates);
    for worker in workers {
        worker.join().unwrap();
    }
    let common_start = result.unwrap();
    assert_eq!(common_start, 100_000_000);
    for _ in 0..2 {
        match packets.recv().unwrap() {
            CaptureEvent::Heartbeat { qpc_100ns, .. } => assert_eq!(qpc_100ns, 100_000_000),
            _ => panic!("commit must emit exact common start"),
        }
    }
    // Neither synthetic source emits audio; in particular remote is idle. The
    // same production start and finish coordinators still cover its exact span.
    assert_eq!(
        complete_interval(common_start, Some(122_000_000), Ok(()), &events, &stop).unwrap(),
        122_000_000
    );
    let mut ended = Vec::new();
    for _ in 0..2 {
        match packets.recv().unwrap() {
            CaptureEvent::Heartbeat { source, qpc_100ns } => {
                ended.push(source);
                assert_eq!(qpc_100ns, 122_000_000);
            }
            _ => panic!("idle interval must end at shared confirmed boundary"),
        }
    }
    assert_eq!(ended, vec![Source::Microphone, Source::Remote]);
}

#[test]
fn late_or_backwards_health_checks_cannot_certify_unknown_silence() {
    assert!(verify_health_continuity(100_000_000, 115_000_000).is_ok());
    assert_eq!(
        verify_health_continuity(100_000_000, 115_000_001).unwrap_err(),
        "meeting_capture_health_gap"
    );
    assert_eq!(
        verify_health_continuity(100_000_000, 99_999_999).unwrap_err(),
        "meeting_capture_health_gap"
    );
}

#[test]
fn healthy_two_point_two_second_interval_has_both_exact_tail_boundaries() {
    let (sender, receiver) = capture_event_queue();
    let stop = AtomicBool::new(true);
    assert_eq!(
        complete_interval(100_000_000, Some(122_000_000), Ok(()), &sender, &stop).unwrap(),
        122_000_000
    );
    let mut sources = Vec::new();
    for _ in 0..2 {
        match receiver.recv().unwrap() {
            CaptureEvent::Heartbeat { source, qpc_100ns } => {
                sources.push(source);
                assert_eq!(qpc_100ns, 122_000_000);
            }
            _ => panic!("healthy stop must publish both common end boundaries"),
        }
    }
    assert_eq!(sources, vec![Source::Microphone, Source::Remote]);
}

#[test]
fn failed_or_uncommitted_capture_never_publishes_healthy_tail() {
    for (start, end, result, expected) in [
        (
            100_000_000,
            Some(122_000_000),
            Err("meeting_capture_next_packet_failure".into()),
            "meeting_capture_next_packet_failure",
        ),
        (100_000_000, None, Ok(()), "meeting_capture_cancelled"),
        (
            0,
            Some(122_000_000),
            Ok(()),
            "meeting_capture_interval_invalid",
        ),
        (
            100_000_000,
            Some(99_000_000),
            Ok(()),
            "meeting_capture_interval_invalid",
        ),
    ] {
        let (sender, receiver) = capture_event_queue();
        let stop = AtomicBool::new(true);
        assert_eq!(
            complete_interval(start, end, result, &sender, &stop).unwrap_err(),
            expected
        );
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn full_terminal_boundary_queue_is_an_explicit_capture_failure() {
    let (sender, _receiver) = capture_event_queue();
    let stop = AtomicBool::new(false);
    for _ in 0..2048 {
        sender
            .try_send(CaptureEvent::Heartbeat {
                source: Source::Remote,
                qpc_100ns: 100_000_000,
            })
            .unwrap();
    }
    assert_eq!(
        complete_interval(100_000_000, Some(122_000_000), Ok(()), &sender, &stop).unwrap_err(),
        "meeting_capture_queue_full"
    );
    assert!(stop.load(Ordering::Acquire));
}
