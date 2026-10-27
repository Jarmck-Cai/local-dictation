"""Stateful OS-boundary fakes: no microphone or actual system mute changes."""

import json

import pytest

from validation.audio import PlaybackMute


class Endpoint:
    def __init__(self, muted=False, volume=0.37):
        self.muted = muted
        self.volume = volume
        self.device_id = "original-speaker"
        self.closed = False
        self.get_error = None
        self.set_errors = []
        self.close_error = None
        self.writes = []

    def get_mute(self):
        if self.get_error:
            raise self.get_error
        return self.muted

    def set_mute(self, value):
        self.writes.append(value)
        # Model even a failing driver call as having changed the device first.
        self.muted = value
        error = self.set_errors.pop(0) if self.set_errors else None
        if error:
            raise error

    def close(self):
        self.closed = True
        if self.close_error:
            raise self.close_error


@pytest.mark.parametrize("original", [False, True])
def test_scope_mutes_during_recording_then_restores_original_without_volume_change(original):
    endpoint = Endpoint(muted=original)
    guard = PlaybackMute(endpoint_factory=lambda: endpoint)
    with guard as entered:
        assert entered is guard
        assert endpoint.muted is True
        assert endpoint.closed is False
        assert guard.report["original_mute"] is original
        assert guard.report["muted"] is True
    assert endpoint.muted is original
    assert endpoint.volume == 0.37
    assert endpoint.closed is True
    assert guard.report["restored"] is True
    assert guard.report["device_id"] == "original-speaker"
    assert json.loads(json.dumps(guard.report))["errors"] == []
    if original:
        assert False not in endpoint.writes


def test_disabled_never_opens_or_changes_a_playback_endpoint():
    def forbidden_factory():
        pytest.fail("disabled scope acquired a system resource")

    with PlaybackMute(enabled=False, endpoint_factory=forbidden_factory) as guard:
        assert guard.report["status"] == "disabled"
    assert guard.report["errors"] == []


@pytest.mark.parametrize("error", [RuntimeError("recording failed"), KeyboardInterrupt()])
def test_recording_exception_is_propagated_after_restore_and_close(error):
    endpoint = Endpoint()
    with pytest.raises(type(error)) as raised:
        with PlaybackMute(endpoint_factory=lambda: endpoint):
            raise error
    assert raised.value is error
    assert endpoint.muted is False
    assert endpoint.closed is True


def test_endpoint_acquisition_failure_does_not_stop_recording_and_is_reported():
    def unavailable():
        raise OSError("no output device")

    recorded = False
    with PlaybackMute(endpoint_factory=unavailable) as guard:
        recorded = True
        assert guard.report["status"] == "failed"
    assert recorded
    assert guard.report["restored"] is None
    assert guard.report["errors"][0]["stage"] == "acquire"
    json.dumps(guard.report)


def test_unknown_original_mute_never_guesses_a_restore_value():
    endpoint = Endpoint(muted=True)
    endpoint.get_error = OSError("GetMute failed")
    with PlaybackMute(endpoint_factory=lambda: endpoint) as guard:
        assert endpoint.muted is True
    assert endpoint.muted is True
    assert endpoint.writes == []
    assert endpoint.closed is True
    assert guard.report["restored"] is None
    assert guard.report["errors"][0]["stage"] == "get_mute"


def test_partial_set_failure_still_restores_the_known_original_state():
    endpoint = Endpoint()
    endpoint.set_errors = [OSError("SetMute failed after write")]
    with PlaybackMute(endpoint_factory=lambda: endpoint) as guard:
        assert endpoint.muted is True
        assert guard.report["status"] == "failed"
    assert endpoint.muted is False
    assert endpoint.closed is True
    assert guard.report["restored"] is True
    assert guard.report["errors"][0]["stage"] == "set_mute"


def test_restore_failure_is_reported_and_does_not_replace_recording_exception():
    endpoint = Endpoint()
    endpoint.set_errors = [None, OSError("restore device unplugged")]
    original = RuntimeError("input stream failed")
    with pytest.raises(RuntimeError) as raised:
        with PlaybackMute(endpoint_factory=lambda: endpoint) as guard:
            raise original
    assert raised.value is original
    assert endpoint.closed is True
    assert guard.report["restored"] is False
    assert guard.report["status"] == "restore_failed"
    assert guard.report["errors"][0]["stage"] == "restore"


def test_output_switch_restores_only_the_endpoint_selected_on_entry():
    old = Endpoint()
    new = Endpoint()
    default = [old]
    with PlaybackMute(endpoint_factory=lambda: default[0]) as guard:
        default[0] = new
        assert old.muted is True
    assert old.muted is False and old.closed is True
    assert new.muted is False and new.closed is False
    assert new.writes == []
    assert guard.report["device_id"] == "original-speaker"


def test_close_failure_does_not_hide_successful_restore_or_body_exception():
    endpoint = Endpoint()
    endpoint.close_error = OSError("release failed")
    original = RuntimeError("input failed")
    with pytest.raises(RuntimeError) as raised:
        with PlaybackMute(endpoint_factory=lambda: endpoint) as guard:
            raise original
    assert raised.value is original
    assert endpoint.muted is False
    assert guard.report["restored"] is True
    assert guard.report["errors"][0]["stage"] == "close"


@pytest.mark.parametrize("stage", ["get", "set"])
def test_keyboard_interrupt_during_entry_closes_and_restores_when_original_known(stage):
    endpoint = Endpoint()
    if stage == "get":
        endpoint.get_error = KeyboardInterrupt()
    else:
        endpoint.set_errors = [KeyboardInterrupt()]
    with pytest.raises(KeyboardInterrupt):
        with PlaybackMute(endpoint_factory=lambda: endpoint):
            pytest.fail("entry interrupt was swallowed")
    assert endpoint.closed is True
    assert endpoint.muted is False


def test_keyboard_interrupt_during_restore_still_closes_and_propagates():
    endpoint = Endpoint()
    endpoint.set_errors = [None, KeyboardInterrupt()]
    with pytest.raises(KeyboardInterrupt):
        with PlaybackMute(endpoint_factory=lambda: endpoint):
            pass
    assert endpoint.closed is True
