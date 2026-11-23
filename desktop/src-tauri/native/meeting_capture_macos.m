#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>

typedef void (*DeviceCallback)(void *, const char *, const char *, bool);
typedef void (*AudioCallback)(void *, uint32_t, uint32_t, uint32_t, uint64_t, const float *);
typedef void (*ErrorCallback)(void *, uint32_t);

enum { CaptureOK, CaptureUnsupported, CapturePermission, CaptureDevice, CaptureStart,
       CaptureFormat, CaptureClock, CaptureInterrupted, CaptureStop, CaptureTimeout };
enum { Microphone, SystemAudio };
enum { MaxChannels = 32, MaxFrames = 8192 };

static uint64_t ticks(CMTime time) {
    if (!CMTIME_IS_NUMERIC(time)) return 0;
    int64_t value = CMTimeConvertScale(time, 10000000, kCMTimeRoundingMethod_RoundHalfAwayFromZero).value;
    return value > 0 ? (uint64_t)value : 0;
}

uint64_t qingshu_macos_meeting_clock(void) {
    return ticks(CMClockGetTime(CMClockGetHostTimeClock()));
}

bool qingshu_macos_meeting_supported(void) {
    if (@available(macOS 15.0, *)) return true;
    return false;
}

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
static NSArray<AVCaptureDevice *> *microphones(void) {
    // The full media-type list includes USB, Bluetooth, and virtual inputs.
    return [AVCaptureDevice devicesWithMediaType:AVMediaTypeAudio];
}
#pragma clang diagnostic pop

void qingshu_macos_meeting_devices(void *context, DeviceCallback callback) {
    @autoreleasepool {
        NSString *defaultID = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeAudio].uniqueID;
        for (AVCaptureDevice *device in microphones()) {
            if (device.connected && device.uniqueID.length > 0) {
                callback(context, device.uniqueID.UTF8String, device.localizedName.UTF8String,
                         [device.uniqueID isEqualToString:defaultID]);
            }
        }
    }
}

API_AVAILABLE(macos(15.0))
@interface QingshuMeetingCapture : NSObject <SCStreamDelegate, SCStreamOutput> {
@public
    dispatch_queue_t queue;
    SCStream *stream;
    void *context;  // Accessed only on queue after start.
    AudioCallback audio;
    ErrorCallback failed;
    dispatch_semaphore_t starting;
    int startStatus; // Written before signaling starting, read after the signal.
}
- (void)startWithMicrophone:(NSString *)identifier;
- (int)stopAndDisconnect;
@end

@implementation QingshuMeetingCapture

- (void)startWithMicrophone:(NSString *)identifier {
    [SCShareableContent getShareableContentExcludingDesktopWindows:NO
                                               onScreenWindowsOnly:NO
                                                 completionHandler:^(SCShareableContent *content, NSError *error) {
        dispatch_async(self->queue, ^{
            if (!self->context) return;  // A timed-out start has already been disconnected.
            SCDisplay *display = nil;
            for (SCDisplay *candidate in content.displays) {
                if (candidate.displayID == CGMainDisplayID()) {
                    display = candidate;
                    break;
                }
            }
            if (error || !display) {
                self->startStatus = !CGPreflightScreenCaptureAccess() ? CapturePermission : CaptureDevice;
                dispatch_semaphore_signal(self->starting);
                return;
            }
            SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display
                                                        excludingApplications:@[]
                                                             exceptingWindows:@[]];
            SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
            config.capturesAudio = YES;
            config.excludesCurrentProcessAudio = YES;
            config.captureMicrophone = YES;
            config.microphoneCaptureDeviceID = identifier;
            config.sampleRate = 48000;
            config.channelCount = 1;
            config.width = 64;
            config.height = 64;
            config.minimumFrameInterval = CMTimeMake(1, 1);
            self->stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:self];
            // Only audio outputs are registered. Screen buffers never enter the app.
            NSError *outputError = nil;
            if (![self->stream addStreamOutput:self type:SCStreamOutputTypeAudio
                           sampleHandlerQueue:self->queue error:&outputError] ||
                ![self->stream addStreamOutput:self type:SCStreamOutputTypeMicrophone
                           sampleHandlerQueue:self->queue error:&outputError]) {
                self->startStatus = CaptureStart;
                dispatch_semaphore_signal(self->starting);
                return;
            }
            [self->stream startCaptureWithCompletionHandler:^(NSError *startError) {
                dispatch_async(self->queue, ^{
                    if (!self->context) {
                        if (!startError) [self->stream stopCaptureWithCompletionHandler:nil];
                        return;
                    }
                    self->startStatus = startError ?
                        (!CGPreflightScreenCaptureAccess() ? CapturePermission : CaptureStart) : CaptureOK;
                    dispatch_semaphore_signal(self->starting);
                });
            }];
        });
    }];
}

- (int)stopAndDisconnect {
    // Allow callbacks already queued by ScreenCaptureKit to drain before
    // releasing the Rust context. After disconnect, late callbacks are inert.
    __block SCStream *current = nil;
    dispatch_sync(queue, ^{ current = self->stream; });
    int status = CaptureOK;
    if (current) {
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block bool failedStop = false;
        [current stopCaptureWithCompletionHandler:^(NSError *error) {
            failedStop = error != nil;
            dispatch_semaphore_signal(done);
        }];
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)))
            status = CaptureTimeout;
        else if (failedStop)
            status = CaptureStop;
    }
    dispatch_sync(queue, ^{
        self->context = NULL;
        if (current) {
            [current removeStreamOutput:self type:SCStreamOutputTypeAudio error:NULL];
            [current removeStreamOutput:self type:SCStreamOutputTypeMicrophone error:NULL];
        }
        // Keep stream alive for the late start-completion handler to stop it.
    });
    return status;
}

- (void)stream:(SCStream *)sender didStopWithError:(NSError *)error {
    dispatch_async(queue, ^{
        if (self->context) self->failed(self->context, CaptureInterrupted);
    });
}

- (void)stream:(SCStream *)sender didOutputSampleBuffer:(CMSampleBufferRef)sample
        ofType:(SCStreamOutputType)type {
    if (!context || (type != SCStreamOutputTypeAudio && type != SCStreamOutputTypeMicrophone)) return;
    CMItemCount frames = CMSampleBufferGetNumSamples(sample);
    if (frames == 0) return;
    CMAudioFormatDescriptionRef format = CMSampleBufferGetFormatDescription(sample);
    const AudioStreamBasicDescription *asbd = format ? CMAudioFormatDescriptionGetStreamBasicDescription(format) : NULL;
    bool float32 = asbd && (asbd->mFormatFlags & kAudioFormatFlagIsFloat) && asbd->mBitsPerChannel == 32;
    bool pcm16 = asbd && (asbd->mFormatFlags & kAudioFormatFlagIsSignedInteger) && asbd->mBitsPerChannel == 16;
    if (!CMSampleBufferDataIsReady(sample) || frames < 0 || frames > MaxFrames || !asbd
        || asbd->mFormatID != kAudioFormatLinearPCM
        || (!float32 && !pcm16)
        || (asbd->mFormatFlags & kAudioFormatFlagIsBigEndian)
        || !(asbd->mFormatFlags & kAudioFormatFlagIsPacked)
        || asbd->mChannelsPerFrame == 0 || asbd->mChannelsPerFrame > MaxChannels
        || !isfinite(asbd->mSampleRate) || asbd->mSampleRate < 8000 || asbd->mSampleRate > 384000) {
        failed(context, CaptureFormat);
        return;
    }
    CMClockRef clock = sender.synchronizationClock;
    uint64_t time = clock ? ticks(CMSyncConvertTime(CMSampleBufferGetPresentationTimeStamp(sample),
                                                   clock, CMClockGetHostTimeClock())) : 0;
    if (!time) {
        failed(context, CaptureClock);
        return;
    }

    struct { UInt32 count; AudioBuffer buffers[MaxChannels]; } storage = {0};
    AudioBufferList *list = (AudioBufferList *)&storage;
    CMBlockBufferRef retained = NULL;
    OSStatus result = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        sample, NULL, list, sizeof(storage), NULL, NULL,
        kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, &retained);
    float mono[MaxFrames] = {0};
    uint32_t channels = 0;
    bool valid = result == noErr && list->mNumberBuffers <= MaxChannels;
    if (valid) {
        for (UInt32 i = 0; i < list->mNumberBuffers; ++i) {
            const AudioBuffer *buffer = &storage.buffers[i];
            UInt32 count = buffer->mNumberChannels;
            size_t bytesPerSample = float32 ? sizeof(float) : sizeof(int16_t);
            if (!count || count > MaxChannels - channels || !buffer->mData ||
                buffer->mDataByteSize < (size_t)frames * count * bytesPerSample) {
                valid = false;
                break;
            }
            for (CMItemCount frame = 0; frame < frames; ++frame) {
                for (UInt32 channel = 0; channel < count; ++channel) {
                    size_t index = (size_t)frame * count + channel;
                    float value = float32 ? ((const float *)buffer->mData)[index]
                        : ((const int16_t *)buffer->mData)[index] / 32768.0f;
                    mono[frame] += value / asbd->mChannelsPerFrame;
                }
            }
            channels += count;
        }
    }
    if (valid && channels == asbd->mChannelsPerFrame) {
        audio(context, type == SCStreamOutputTypeMicrophone ? Microphone : SystemAudio,
              (uint32_t)asbd->mSampleRate, (uint32_t)frames, time, mono);
    } else {
        failed(context, CaptureFormat);
    }
    if (retained) CFRelease(retained);
}
@end

void *qingshu_macos_meeting_start(const char *microphoneID, void *context,
                                 AudioCallback audio, ErrorCallback failed, int *code) {
    @autoreleasepool {
        if (@available(macOS 15.0, *)) {
            NSString *identifier = [NSString stringWithUTF8String:microphoneID];
            bool found = false;
            for (AVCaptureDevice *device in microphones())
                if (device.connected && [device.uniqueID isEqualToString:identifier]) found = true;
            if (!found) { *code = CaptureDevice; return NULL; }
            if (!CGPreflightScreenCaptureAccess() && !CGRequestScreenCaptureAccess()) {
                *code = CapturePermission;
                return NULL;
            }
            QingshuMeetingCapture *capture = [QingshuMeetingCapture new];
            capture->queue = dispatch_queue_create("com.localdictation.meeting.audio", DISPATCH_QUEUE_SERIAL);
            capture->starting = dispatch_semaphore_create(0);
            capture->context = context;
            capture->audio = audio;
            capture->failed = failed;
            [capture startWithMicrophone:identifier];
            if (dispatch_semaphore_wait(capture->starting, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC))) {
                *code = CaptureTimeout;
            } else {
                *code = capture->startStatus;
                if (*code == CaptureOK) return (__bridge_retained void *)capture;
            }
            [capture stopAndDisconnect];
            return NULL;
        }
        *code = CaptureUnsupported;
        return NULL;
    }
}

int qingshu_macos_meeting_stop(void *handle) {
    @autoreleasepool {
        if (@available(macOS 15.0, *)) {
            QingshuMeetingCapture *capture = (__bridge_transfer QingshuMeetingCapture *)handle;
            return [capture stopAndDisconnect];
        }
        return CaptureUnsupported;
    }
}
