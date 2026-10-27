"""Temporarily mute one Windows playback endpoint for a recording scope.

Only the mute flag changes. This is not source separation, and Python cannot
guarantee restoration after process termination or power loss. Use the scope on
one thread so its COM initialization and release stay on that same thread.
"""

import ctypes
import sys
import uuid


class PlaybackMute:
    """Best-effort mute/restore with JSON-safe diagnostics and no hidden retry."""

    def __init__(self, enabled=True, endpoint_factory=None):
        self.enabled = enabled
        self._factory = endpoint_factory or _WindowsPlaybackEndpoint
        self._endpoint = None
        self.report = {
            "enabled": bool(enabled),
            "status": "pending" if enabled else "disabled",
            "original_mute": None,
            "muted": False,
            "restored": None,
            "device_id": None,
            "errors": [],
        }

    def _error(self, stage, error):
        self.report["errors"].append({"stage": stage, "error": f"{type(error).__name__}: {error}"})

    def __enter__(self):
        if not self.enabled:
            return self
        stage = "acquire"
        try:
            self._endpoint = self._factory()
            self.report["device_id"] = getattr(self._endpoint, "device_id", None)
            stage = "get_mute"
            self.report["original_mute"] = bool(self._endpoint.get_mute())
            stage = "set_mute"
            self._endpoint.set_mute(True)
            self.report["muted"] = True
            self.report["status"] = "muted"
        except Exception as error:
            self._error(stage, error)
            self.report["status"] = "failed"
        except BaseException:
            # A with statement never calls __exit__ when __enter__ is interrupted.
            self._finish()
            raise
        return self

    def _finish(self):
        endpoint, self._endpoint = self._endpoint, None
        if endpoint is None:
            return
        try:
            if self.report["original_mute"] is not None:
                try:
                    # Try even when the first SetMute raised: a driver may have
                    # applied it before reporting an error. Never reacquire default.
                    endpoint.set_mute(self.report["original_mute"])
                    self.report["restored"] = True
                    if not self.report["errors"]:
                        self.report["status"] = "restored"
                except Exception as error:
                    self.report["restored"] = False
                    self.report["status"] = "restore_failed"
                    self._error("restore", error)
        finally:
            try:
                endpoint.close()
            except Exception as error:
                self._error("close", error)

    def __exit__(self, exc_type, exc, traceback):
        self._finish()
        return False


class _GUID(ctypes.Structure):
    _fields_ = [("data1", ctypes.c_uint32), ("data2", ctypes.c_uint16),
                ("data3", ctypes.c_uint16), ("data4", ctypes.c_ubyte * 8)]

    @classmethod
    def parse(cls, value):
        return cls.from_buffer_copy(uuid.UUID(value).bytes_le)


_HRESULT = ctypes.c_int32
_BOOL = ctypes.c_int32
_DWORD = ctypes.c_uint32
_VOID = ctypes.c_void_p


def _check_hr(result, operation):
    # HRESULT is signed 32 bits on both Win32 and Win64; S_FALSE is success.
    if ctypes.c_int32(result).value < 0:
        raise OSError(f"{operation} failed (HRESULT 0x{result & 0xffffffff:08X})")


def _com_call(pointer, index, restype, argtypes, *args):
    vtable = ctypes.cast(pointer, ctypes.POINTER(ctypes.POINTER(_VOID))).contents
    function = ctypes.WINFUNCTYPE(restype, _VOID, *argtypes)(vtable[index])
    return function(pointer, *args)


class _WindowsPlaybackEndpoint:
    """Own the original eRender/eMultimedia endpoint and its COM references.

    GUIDs, ABI signatures and vtable slots follow Microsoft's SDK headers:
    https://github.com/microsoft/win32metadata/blob/main/generation/WinSDK/RecompiledIdlHeaders/um/mmdeviceapi.h
    https://github.com/microsoft/win32metadata/blob/main/generation/WinSDK/RecompiledIdlHeaders/um/endpointvolume.h
    """

    def __init__(self):
        if sys.platform != "win32":
            raise OSError("Playback mute requires Windows Core Audio")
        self._enumerator = _VOID()
        self._device = _VOID()
        self._volume = _VOID()
        self._com_initialized = False
        self.device_id = None
        self._ole = ctypes.WinDLL("ole32")
        self._ole.CoInitializeEx.argtypes = [_VOID, _DWORD]
        self._ole.CoInitializeEx.restype = _HRESULT
        self._ole.CoUninitialize.argtypes = []
        self._ole.CoUninitialize.restype = None
        self._ole.CoCreateInstance.argtypes = [ctypes.POINTER(_GUID), _VOID, _DWORD,
                                                ctypes.POINTER(_GUID), ctypes.POINTER(_VOID)]
        self._ole.CoCreateInstance.restype = _HRESULT
        self._ole.CoTaskMemFree.argtypes = [_VOID]
        self._ole.CoTaskMemFree.restype = None
        try:
            result = self._ole.CoInitializeEx(None, 0)  # COINIT_MULTITHREADED
            # The host may already use STA. Reuse its apartment without owning
            # an initialization count; all other failed HRESULTs abort acquire.
            if result & 0xffffffff != 0x80010106:  # RPC_E_CHANGED_MODE
                _check_hr(result, "CoInitializeEx")
                self._com_initialized = True
            clsid = _GUID.parse("BCDE0395-E52F-467C-8E3D-C4579291692E")
            iid = _GUID.parse("A95664D2-9614-4F35-A746-DE8DB63617E6")
            _check_hr(self._ole.CoCreateInstance(ctypes.byref(clsid), None, 1,
                      ctypes.byref(iid), ctypes.byref(self._enumerator)), "CoCreateInstance")
            # IMMDeviceEnumerator::GetDefaultAudioEndpoint is slot 4 after IUnknown.
            _check_hr(_com_call(self._enumerator, 4, _HRESULT,
                      [ctypes.c_int32, ctypes.c_int32, ctypes.POINTER(_VOID)],
                      0, 1, ctypes.byref(self._device)), "GetDefaultAudioEndpoint")
            identifier = _VOID()
            try:
                # IMMDevice::GetId slot 5 allocates an LPWSTR with CoTaskMemAlloc.
                _check_hr(_com_call(self._device, 5, _HRESULT,
                          [ctypes.POINTER(_VOID)], ctypes.byref(identifier)), "GetId")
                self.device_id = ctypes.wstring_at(identifier)
            finally:
                if identifier:
                    self._ole.CoTaskMemFree(identifier)
            volume_iid = _GUID.parse("5CDF2C82-841E-4546-9722-0CF74078229A")
            # IMMDevice::Activate slot 3; CLSCTX_ALL=23, NULL activation params.
            _check_hr(_com_call(self._device, 3, _HRESULT,
                      [ctypes.POINTER(_GUID), _DWORD, _VOID, ctypes.POINTER(_VOID)],
                      ctypes.byref(volume_iid), 23, None, ctypes.byref(self._volume)), "Activate")
        except BaseException:
            self.close()
            raise

    def get_mute(self):
        value = _BOOL()
        _check_hr(_com_call(self._volume, 15, _HRESULT,
                  [ctypes.POINTER(_BOOL)], ctypes.byref(value)), "GetMute")
        return bool(value.value)

    def set_mute(self, value):
        _check_hr(_com_call(self._volume, 14, _HRESULT, [_BOOL, ctypes.POINTER(_GUID)],
                  bool(value), None), "SetMute")

    def close(self):
        # Reverse ownership order, before balancing our own COM initialization.
        try:
            for name in ("_volume", "_device", "_enumerator"):
                pointer = getattr(self, name)
                if pointer:
                    setattr(self, name, _VOID())
                    _com_call(pointer, 2, ctypes.c_uint32, [])  # IUnknown::Release
        finally:
            if self._com_initialized:
                self._com_initialized = False
                self._ole.CoUninitialize()
