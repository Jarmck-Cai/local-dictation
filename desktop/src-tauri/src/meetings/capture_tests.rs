use super::super::queue::capture_event_queue;
use super::*;

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
