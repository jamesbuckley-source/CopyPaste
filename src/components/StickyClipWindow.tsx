import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { AnimatedImage } from "./AnimatedImage";
import { IconClose } from "./icons";
import { clipFullImageUrl, clipLabel, clipMotionUrl } from "../lib/format";
import { errorMessage, getClip, mediaUrl } from "../lib/ipc";
import type { ClipDetail } from "../lib/types";
import { MaskedSecret } from "./SecretPreview";
import { useSettings } from "../hooks/useSettings";
import { applyTheme } from "../lib/theme";

export function StickyClipWindow({ id }: { id: number | null }) {
  const [clip, setClip] = useState<ClipDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { settings } = useSettings();

  useEffect(() => {
    if (!settings) return;
    applyTheme(settings.theme);
    document.documentElement.style.setProperty("--ui-scale", String(settings.textScale / 100));
  }, [settings]);

  useEffect(() => {
    let active = true;
    let request = 0;
    let unlisten: (() => void) | undefined;
    const refresh = () => {
      if (id === null) return;
      const currentRequest = ++request;
      getClip(id)
        .then((detail) => {
          if (active && currentRequest === request) {
            setClip(detail);
            setError(null);
          }
        })
        .catch((cause) => {
          if (active && currentRequest === request) setError(errorMessage(cause));
        });
    };
    refresh();
    listen("history:changed", refresh)
      .then((stop) => {
        if (active) unlisten = stop;
        else stop();
      })
      .catch((cause) => {
        if (active) setError(errorMessage(cause));
      });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [id]);

  const close = useCallback(() => {
    void getCurrentWindow()
      .close()
      .catch((cause) => setError(errorMessage(cause)));
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [close]);

  const text = clip?.text;
  const imageUrl = clip ? clipFullImageUrl(clip) : null;
  const motionUrl = clip ? clipMotionUrl(clip) : null;

  return (
    <main className="flex h-full min-h-0 flex-col overflow-hidden border border-edge bg-panel text-ink">
      <header
        data-tauri-drag-region
        className="flex h-9 shrink-0 items-center gap-2 border-b border-edge bg-panel-2 px-2"
      >
        <span
          data-tauri-drag-region
          className="min-w-0 flex-1 truncate text-[11px] font-medium text-ink-dim"
        >
          {clip ? `Sticky ${clipLabel(clip)}` : "Sticky clip"}
        </span>
        <button
          type="button"
          onClick={close}
          aria-label="Close sticky clip"
          title="Close"
          className="flex size-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-panel-3 hover:text-ink"
        >
          <IconClose className="size-3.5" />
        </button>
      </header>

      {error ? (
        <p role="alert" className="m-auto max-w-full p-4 text-xs text-danger">
          {error}
        </p>
      ) : id === null ? (
        <p role="alert" className="m-auto p-4 text-xs text-danger">
          This sticky clip link is invalid.
        </p>
      ) : !clip ? (
        <p className="m-auto p-4 text-xs text-ink-faint">Loading clip…</p>
      ) : clip.kind === "text" ? (
        <div className="min-h-0 flex-1 overflow-auto p-3">
          {clip.secretLevel && (text == null || text === "") ? (
            <p className="whitespace-pre-wrap break-words font-mono text-xs text-ink-faint">
              Unlock the vault in CopyPaste to view this clip.
            </p>
          ) : (
            <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-ink">
              {clip.secretLevel && text != null ? (
                <MaskedSecret text={text} />
              ) : (
                text ?? clip.preview
              )}
            </pre>
          )}
        </div>
      ) : clip.kind === "image" ? (
        <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-3">
          {clip.mediaLocked ? (
            <p className="text-xs text-ink-faint">Unlock the vault in CopyPaste to view this image.</p>
          ) : imageUrl ? (
            clip.video ? (
              <video
                src={motionUrl ?? imageUrl}
                poster={clip.thumbFile ? mediaUrl(clip.thumbFile) : undefined}
                autoPlay
                controls
                loop
                muted
                playsInline
                className="max-h-full max-w-full object-contain"
              />
            ) : (
              <AnimatedImage
                still={clip.imageFile ? mediaUrl(clip.imageFile) : null}
                animated={motionUrl}
                loop
                alt="Sticky clipboard image"
                loading="eager"
                className="max-h-full max-w-full object-contain"
              />
            )
          ) : (
            <p className="text-xs text-ink-faint">The image file is no longer available.</p>
          )}
        </div>
      ) : (
        <ul className="min-h-0 flex-1 space-y-1 overflow-auto p-3 text-xs text-ink">
          {clip.filePaths.length > 0 ? (
            clip.filePaths.map((path, index) => (
              <li key={`${index}:${path}`} className="break-all font-mono">
                {path}
              </li>
            ))
          ) : (
            <li className="text-ink-faint">No file paths are available.</li>
          )}
        </ul>
      )}
    </main>
  );
}
