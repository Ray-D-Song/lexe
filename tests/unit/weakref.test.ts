it("WeakRef keeps target alive through the current job (KeepDuringJob)", async () => {
  const ref = new WeakRef({ ok: 42 });
  expect(ref.deref()?.ok).toBe(42);
  await Promise.resolve();
  expect(ref.deref()?.ok).toBe(42);
  await new Promise((resolve) => setTimeout(resolve, 0));
  if (typeof (globalThis as { __gc?: () => void }).__gc === "function") {
    (globalThis as { __gc: () => void }).__gc();
  }
  expect(ref.deref()).toBe(undefined);
});
