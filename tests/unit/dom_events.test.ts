describe("DOM Events", () => {
  it("supports Event phases, preventDefault, stopImmediatePropagation, and composedPath", () => {
    const ev = new Event("submit", { bubbles: true, cancelable: true, composed: true });
    expect(ev.type).toBe("submit");
    expect(ev.bubbles).toBe(true);
    expect(ev.cancelable).toBe(true);
    expect(ev.composed).toBe(true);
    expect(ev.eventPhase).toBe(Event.NONE);
    expect(ev.defaultPrevented).toBe(false);
    expect(ev.returnValue).toBe(true);
    expect(ev.isTrusted).toBe(false);
    expect(typeof ev.timeStamp).toBe("number");
    expect(ev.target).toBeNull();
    expect(ev.currentTarget).toBeNull();
    expect(ev.composedPath()).toEqual([]);

    const target = new EventTarget();
    const seen: string[] = [];
    let phaseDuringDispatch = -1;
    let pathDuringDispatch: unknown[] = [];

    target.addEventListener("submit", (e: any) => {
      seen.push("first");
      phaseDuringDispatch = e.eventPhase;
      pathDuringDispatch = e.composedPath();
      expect(e.target).toBe(target);
      expect(e.srcElement).toBe(target);
      expect(e.currentTarget).toBe(target);
      e.preventDefault();
      e.stopImmediatePropagation();
    });

    target.addEventListener("submit", () => {
      seen.push("second");
    });

    const notCancelled = target.dispatchEvent(ev);
    expect(notCancelled).toBe(false);
    expect(seen).toEqual(["first"]);
    expect(phaseDuringDispatch).toBe(Event.AT_TARGET);
    expect(pathDuringDispatch).toEqual([target]);
    expect(ev.defaultPrevented).toBe(true);
    expect(ev.returnValue).toBe(false);
    expect(ev.cancelBubble).toBe(true);
    expect(ev.target).toBe(target);
    expect(ev.currentTarget).toBeNull();
    expect(ev.eventPhase).toBe(Event.NONE);
  });

  it("ignores preventDefault when cancelable is false and supports ErrorEvent & CloseEvent", () => {
    const nonCancelable = new CustomEvent("ping", { detail: { ok: true }, bubbles: true });
    nonCancelable.preventDefault();
    expect(nonCancelable.defaultPrevented).toBe(false);
    expect(nonCancelable.bubbles).toBe(true);
    expect(nonCancelable.detail).toEqual({ ok: true });
    expect(nonCancelable instanceof Event).toBe(true);

    const boom = new TypeError("kaboom");
    const errEv = new ErrorEvent("error", {
      message: "failed",
      filename: "app.ts",
      lineno: 42,
      colno: 7,
      error: boom,
      cancelable: true,
    });
    expect(errEv instanceof Event).toBe(true);
    expect(errEv.type).toBe("error");
    expect(errEv.message).toBe("failed");
    expect(errEv.filename).toBe("app.ts");
    expect(errEv.lineno).toBe(42);
    expect(errEv.colno).toBe(7);
    expect(errEv.error).toBe(boom);

    const defaultErr = new ErrorEvent("error");
    expect(defaultErr.message).toBe("");
    expect(defaultErr.filename).toBe("");
    expect(defaultErr.lineno).toBe(0);
    expect(defaultErr.colno).toBe(0);
    expect(defaultErr.error).toBeUndefined();

    const et = new EventTarget();
    let handled = false;
    et.addEventListener("error", (e: any) => {
      handled = true;
      e.preventDefault();
    });
    // Unhandled "error" on EventTarget must not throw; preventDefault cancels dispatchEvent return.
    expect(new EventTarget().dispatchEvent(new ErrorEvent("error"))).toBe(true);
    expect(et.dispatchEvent(errEv)).toBe(false);
    expect(handled).toBe(true);

    const closeEv = new CloseEvent("close", {
      wasClean: true,
      code: 1000,
      reason: "normal closure",
    });
    expect(closeEv instanceof Event).toBe(true);
    expect(closeEv.wasClean).toBe(true);
    expect(closeEv.code).toBe(1000);
    expect(closeEv.reason).toBe("normal closure");

    const defaultClose = new CloseEvent("close");
    expect(defaultClose.wasClean).toBe(false);
    expect(defaultClose.code).toBe(0);
    expect(defaultClose.reason).toBe("");
  });
});
