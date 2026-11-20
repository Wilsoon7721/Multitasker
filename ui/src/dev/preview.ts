export async function loadPreviewBackend(): Promise<void> {
  // Only in a plain browser during development; a no-op inside Tauri and in builds.
  if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
    await import('./mockBackend');
    document.documentElement.dataset.preview = 'true';
  }
}
