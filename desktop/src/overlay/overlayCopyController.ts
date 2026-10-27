export type OverlayCopyState = "idle" | "copying" | "copied" | "failed";

export interface OverlayCopyNotice {
  failures: number;
  failure_generation: number;
  capture_generation: number;
}

interface OverlayCopyDependencies {
  copy: () => Promise<void>;
  dismiss: (failureGeneration: number) => Promise<void>;
  onCopyState: (state: OverlayCopyState) => void;
  schedule?: (callback: () => void, delay: number) => () => void;
}

export class OverlayCopyController {
  private notice?: OverlayCopyNotice;
  private revision = 0;
  private state: OverlayCopyState = "idle";
  private cancelAutoClose?: () => void;
  private readonly schedule: NonNullable<OverlayCopyDependencies["schedule"]>;

  constructor(private readonly dependencies: OverlayCopyDependencies) {
    this.schedule =
      dependencies.schedule ??
      ((callback, delay) => {
        const timer = globalThis.setTimeout(callback, delay);
        return () => globalThis.clearTimeout(timer);
      });
  }

  show(notice: OverlayCopyNotice) {
    if (notice.failures === 0) {
      this.hide();
      return;
    }
    if (
      this.notice?.failure_generation === notice.failure_generation &&
      this.notice.capture_generation === notice.capture_generation &&
      this.notice.failures === notice.failures
    ) {
      return;
    }
    this.invalidate();
    this.notice = { ...notice };
    this.setState("idle");
  }

  hide() {
    this.invalidate();
    this.notice = undefined;
    this.setState("idle");
  }

  private invalidate() {
    this.revision += 1;
    this.cancelAutoClose?.();
    this.cancelAutoClose = undefined;
  }

  private setState(state: OverlayCopyState) {
    if (this.state === state) return;
    this.state = state;
    this.dependencies.onCopyState(state);
  }

  async copy() {
    if (!this.notice || this.state === "copying") return;
    this.invalidate();
    const revision = this.revision;
    this.setState("copying");
    try {
      await this.dependencies.copy();
    } catch {
      if (this.revision === revision) this.setState("failed");
      return;
    }
    if (this.revision !== revision) return;
    this.setState("copied");
    // Start the feedback interval only after the copy actually succeeded. The
    // revision check also rejects a timer already queued before cancellation.
    this.cancelAutoClose = this.schedule(() => {
      if (this.revision === revision) void this.dismiss();
    }, 1500);
  }

  async dismiss() {
    if (!this.notice) return;
    const generation = this.notice.failure_generation;
    this.invalidate();
    if (this.state === "copying") this.setState("idle");
    try {
      // The native command acknowledges this exact failure version and restores
      // any active capture/processing phase. Never hide the window directly.
      await this.dependencies.dismiss(generation);
    } catch {
      // Keep the card and its manual controls available if IPC cannot dismiss.
    }
  }
}
