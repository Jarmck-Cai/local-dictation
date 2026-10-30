/** Complete root `meetings` subtree, registered by i18n/index.ts. */
export const meetingMessages = {
  en: {
    dateUnavailable: "Recording date unavailable",
    nav: "Meetings",
    title: "Meeting recording",
    beta: "Beta",
    description:
      "Try dual-track recording and local transcription after your meeting.",
    betaHint:
      "Speaker playback may cause duplicate text or incorrect source attribution. Speaker identification is not yet guaranteed.",
    newMeeting: "New recording",
    meetingTitle: "Meeting title (optional)",
    titlePlaceholder: "Give this meeting a title",
    untitled: "Untitled meeting",
    microphone: "Microphone",
    playback: "Computer audio · Playback device",
    chooseMicrophone: "Choose a microphone",
    choosePlayback: "Choose a playback device",
    defaultDevice: "{{name}} (communications default)",
    playbackHint:
      "All sounds played through the selected playback device will be recorded, including sounds from other apps. Choose the same device your meeting uses.",
    deviceHint:
      "The first available microphone and playback device are selected automatically. Check that playback matches your meeting. Devices stay fixed during recording.",
    refreshDevices: "Refresh devices",
    loadingDevices: "Finding audio devices…",
    noMicrophone: "No microphone found. Connect one, then refresh devices.",
    noPlayback:
      "No playback device found. Connect speakers or headphones, then refresh devices.",
    start: "Start recording",
    starting: "Starting…",
    current: "Current meeting",
    finish: "Finish and transcribe",
    finishing: "Saving and starting transcription…",
    recordingHint:
      "Recording continues in the background. Return here to finish and transcribe. Regular dictation is paused while this meeting is recording.",
    processingHint:
      "Transcribing on this computer. You can cancel and retry later; your recording is kept.",
    cancelTranscription: "Cancel transcription",
    cancelling: "Cancelling…",
    cancelHint:
      "Cancellation may take a moment while the current section finishes.",
    activeHint:
      "Finish the current recording or transcription before starting another meeting.",
    progress: "{{percent}}% transcribed",
    progressLabel: "Transcription progress",
    checkingCurrent: "Checking the current meeting…",
    recent: "Recent meetings",
    refresh: "Refresh",
    loading: "Loading meetings…",
    empty:
      "No meetings yet. Choose your audio devices above to start your first recording.",
    selectHint: "Select a meeting to view its transcript and export options.",
    detail: "Meeting details",
    recordedWith: "Audio devices",
    transcript: "Transcript",
    loadingTranscript: "Loading transcript…",
    noTranscript:
      "No transcript yet. The saved recording can be transcribed below.",
    waitingTranscript:
      "The transcript will be available here when processing finishes.",
    noSpeech: "No speech was recognized in this recording.",
    partial:
      "This transcript is incomplete. You can export the available text or retry transcription from the saved recording.",
    warnings:
      "Check the original recording if the text looks incomplete or unexpected.",
    warningIncompleteRecording:
      "Recording was interrupted. Saved audio and text are kept, but they may not cover the entire meeting.",
    captureDiagnostic: "Diagnostic code: {{code}}",
    captureReasons: {
      stateSave:
        "Recording stopped because its saved state could not be updated.",
      fileAccess:
        "Recording stopped because access to its state file was blocked or conflicted with another operation.",
      device: "Recording stopped because an audio device became unavailable.",
      queue:
        "Recording stopped because audio could not be saved fast enough and the capture queue filled up.",
      capture: "Recording stopped because audio capture failed.",
      storage:
        "Recording stopped because local audio or recording data could not be read or written.",
      unknown:
        "The original reason for this recording interruption was not saved.",
      unknownSaved:
        "Recording was interrupted by an unrecognized error. Diagnostic information was saved.",
    },
    warningInputClamped:
      "Some input audio was too loud, which may reduce transcription accuracy. Replay the recording to check.",
    warningAttributionExperimental:
      "Source attribution is experimental. Speaker playback may cause duplicate text or incorrect attribution; speaker identification is not guaranteed.",
    source: { microphone: "Microphone", remote: "Computer audio" },
    retry: "Retry",
    transcribe: "Transcribe recording",
    retryTranscription: "Retry transcription",
    transcribing: "Starting transcription…",
    savedHint: "Your recording is saved locally and can be transcribed again.",
    exportAudio: "Export audio",
    exportText: "Export text",
    exporting: "Exporting…",
    audioSaveTitle: "Save meeting audio",
    textSaveTitle: "Save meeting transcript",
    audioFile: "WAV audio",
    textFile: "Text document",
    audioExportHint:
      "Exported audio has your microphone on the left channel and playback audio on the right.",
    exported: "Export saved.",
    delete: "Delete meeting",
    deleting: "Deleting…",
    deleteQuestion:
      "Delete this meeting's local audio and transcript? This cannot be undone.",
    confirmDelete: "Delete audio and transcript",
    keep: "Keep meeting",
    retention:
      "Meeting audio and text stay on this computer for seven days. Export anything you want to keep.",
    status: {
      recording: "Recording",
      paused: "Recording stopped",
      recorded: "Ready to transcribe",
      interrupted: "Recording interrupted",
      processing: "Transcribing",
      complete: "Complete",
      cancelled: "Transcription cancelled",
      failed: "Needs attention",
    },
    errors: {
      generic: "This action could not be completed. Please retry.",
      busy: "Another recording or transcription is in progress. Finish it, then retry.",
      devices:
        "The selected audio devices are unavailable. Refresh and choose both devices again.",
      capture:
        "Recording could not be completed. Check your audio devices. Any saved audio remains available.",
      storage:
        "The local recording could not be read or saved. Check available storage and retry.",
      restoreBlocked:
        "The recording state file is temporarily inaccessible. Saved audio is kept. Close anything using the file, then retry.",
      restoreConflict:
        "The recording state recovery information is missing, invalid or conflicting. Saved audio is kept; the recovery information needs to be checked before continuing.",
      model:
        "Local transcription is unavailable. Check that your speech model is ready, then retry. The saved audio is kept.",
      modelUnsupported:
        "Meeting transcription currently supports SenseVoice. Select it in model settings, then retry. Your recording is kept.",
      processing:
        "Transcription could not finish. Retry from the saved recording.",
      export:
        "The file could not be saved there. Choose another save location and retry.",
      missing: "This meeting is no longer available. Refresh the meeting list.",
      unsupported: "Meeting recording is not supported on this device.",
    },
  },
  zh: {
    dateUnavailable: "录制时间暂不可用",
    captureDiagnostic: "诊断码：{{code}}",
    captureReasons: {
      stateSave: "录制状态无法保存，录音已中断。",
      fileAccess: "录制状态文件访问被阻止或发生冲突，录音已中断。",
      device: "音频设备不可用，录音已中断。",
      queue: "音频未能及时保存，采集队列已满，录音已中断。",
      capture: "音频采集发生异常，录音已中断。",
      storage: "本地音频或录制数据读写失败，录音已中断。",
      unknown: "原始中断原因未保存。",
      unknownSaved: "录音因未识别的异常中断，诊断信息已保存。",
    },
    nav: "会议",
    title: "会议录音",
    beta: "Beta 试用",
    description: "本版用于双轨录制和会后本机转录试用。",
    betaHint: "外放可能造成文字重复或来源归属不准，尚未保证说话人识别。",
    newMeeting: "新建录音",
    meetingTitle: "会议名称（选填）",
    titlePlaceholder: "为这场会议起个名字",
    untitled: "未命名会议",
    microphone: "麦克风",
    playback: "电脑声音 · 播放设备",
    chooseMicrophone: "请选择麦克风",
    choosePlayback: "请选择播放设备",
    defaultDevice: "{{name}}（默认通话设备）",
    playbackHint:
      "所选播放设备发出的全部声音都会被录下，包括其他应用的声音。请选择会议正在使用的播放设备。",
    deviceHint:
      "已默认选择首个麦克风和播放设备，请核对播放设备与会议一致。录音期间保持使用这些设备。",
    refreshDevices: "刷新设备",
    loadingDevices: "正在查找声音设备…",
    noMicrophone: "没有找到麦克风，请连接后刷新设备。",
    noPlayback: "没有找到播放设备，请连接扬声器或耳机后刷新设备。",
    start: "开始录音",
    starting: "正在开始…",
    current: "当前会议",
    finish: "结束并转录",
    finishing: "正在保存并启动转录…",
    recordingHint:
      "录音可在后台继续，会议结束后回到这里结束并转录。会议录制期间暂停普通听写。",
    processingHint: "正在本机转录。可以取消后稍后重试，录音会保留。",
    cancelTranscription: "取消转录",
    cancelling: "正在取消…",
    cancelHint: "取消需要稍等片刻，当前片段完成后会停止。",
    activeHint: "请先结束当前录音或转录，再开始下一场会议。",
    progress: "已转录 {{percent}}%",
    progressLabel: "转录进度",
    checkingCurrent: "正在检查当前会议…",
    recent: "近期会议",
    refresh: "刷新",
    loading: "正在读取会议…",
    empty: "还没有会议记录。在上方选择声音设备，即可开始第一场录音。",
    selectHint: "选择一场会议，查看文字和导出选项。",
    detail: "会议详情",
    recordedWith: "录音设备",
    transcript: "转录文字",
    loadingTranscript: "正在读取文字…",
    noTranscript: "还没有转录文字，可以在下方转录已保存的录音。",
    waitingTranscript: "转录完成后，文字会显示在这里。",
    noSpeech: "这段录音没有识别到讲话。",
    partial: "这份文字尚未转录完整。可以导出已有文字，或从保留的录音重新转录。",
    warnings: "如文字有缺失或异常，可回听录音核对。",
    warningIncompleteRecording:
      "录音曾中断。已保存的录音和文字会保留，但可能没有覆盖整场会议。",
    warningInputClamped:
      "部分输入声音过响，可能影响识别准确性，可回听录音核对。",
    warningAttributionExperimental:
      "声音来源归属仍在试验中。外放可能造成文字重复或归属不准，尚未保证说话人识别。",
    source: { microphone: "麦克风", remote: "电脑声音" },
    retry: "重试",
    transcribe: "转录录音",
    retryTranscription: "重新转录",
    transcribing: "正在启动转录…",
    savedHint: "录音已保存在本机，可以重新转录。",
    exportAudio: "导出录音",
    exportText: "导出文字",
    exporting: "正在导出…",
    audioSaveTitle: "保存会议录音",
    textSaveTitle: "保存会议文字",
    audioFile: "WAV 录音",
    textFile: "文本文档",
    audioExportHint: "导出的录音左声道为麦克风，右声道为播放设备的声音。",
    exported: "已保存导出文件。",
    delete: "删除会议",
    deleting: "正在删除…",
    deleteQuestion: "删除这场会议在本机的录音和文字？删除后无法恢复。",
    confirmDelete: "删除录音和文字",
    keep: "保留会议",
    retention: "会议录音和文字在本机保留七天，需要长期保留的内容请及时导出。",
    status: {
      recording: "正在录音",
      paused: "录音已停止",
      recorded: "等待转录",
      interrupted: "录音已中断",
      processing: "正在转录",
      complete: "转录完成",
      cancelled: "已取消转录",
      failed: "需要处理",
    },
    errors: {
      generic: "暂时无法完成，请重试。",
      busy: "还有录音或转录正在进行，请结束后重试。",
      devices: "所选声音设备当前不可用，请刷新并重新选择两台设备。",
      capture: "录音未能正常完成，请检查声音设备。已经保存的录音仍会保留。",
      storage: "无法读取或保存本机录音，请检查可用存储空间后重试。",
      restoreBlocked:
        "录制状态文件暂时无法访问，已保存的原音仍保留。请关闭占用文件的程序后重试。",
      restoreConflict:
        "录制状态的恢复信息缺失、无效或互相冲突。原音仍保留，请先检查恢复信息再继续。",
      model:
        "本机转录暂时不可用，请确认语音模型已就绪后重试。已保存的录音会保留。",
      modelUnsupported:
        "会议转录目前支持 SenseVoice，请在模型设置中选择后重试。已保存的录音会保留。",
      processing: "转录未能完成，可以使用已保存的录音重新转录。",
      export: "无法保存到所选位置，请换一个保存位置后重试。",
      missing: "这场会议已不可用，请刷新会议列表。",
      unsupported: "此设备暂不支持会议录音。",
    },
  },
};
