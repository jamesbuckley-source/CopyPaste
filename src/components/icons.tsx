/**
 * Inline SVG icons.
 *
 * Hand-rolled rather than pulled from an icon package: the set is tiny, and this
 * keeps the bundle free of an extra dependency for eight glyphs.
 */

import type { ReactNode } from "react";

interface IconProps {
  className?: string;
}

function frame(className: string | undefined, children: ReactNode) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

export function IconSearch({ className }: IconProps) {
  return frame(
    className,
    <>
      <circle cx="7" cy="7" r="4.2" />
      <path d="M10.4 10.4 14 14" />
    </>,
  );
}

export function IconSettings({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M12.7 6.61l1.53 0.46l0 1.86l-1.53 0.46l-0.39 0.95l0.76 1.41l-1.32 1.32l-1.41 -0.76l-0.95 0.39l-0.46 1.53l-1.86 0l-0.46 -1.53l-0.95 -0.39l-1.41 0.76l-1.32 -1.32l0.76 -1.41l-0.39 -0.95l-1.53 -0.46l0 -1.86l1.53 -0.46l0.39 -0.95l-0.76 -1.41l1.32 -1.32l1.41 0.76l0.95 -0.39l0.46 -1.53l1.86 0l0.46 1.53l0.95 0.39l1.41 -0.76l1.32 1.32l-0.76 1.41z" />
      <circle cx="8" cy="8" r="2.1" />
    </>,
  );
}

export function IconArchive({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="2.2" y="2.6" width="11.6" height="2.8" rx="0.8" />
      <path d="M3.2 5.4v7a1 1 0 0 0 1 1h7.6a1 1 0 0 0 1-1v-7" />
      <path d="M6.5 8.2h3" />
    </>,
  );
}

export function IconUnarchive({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="2.2" y="2.6" width="11.6" height="2.8" rx="0.8" />
      <path d="M3.2 5.4v7a1 1 0 0 0 1 1h7.6a1 1 0 0 0 1-1v-7" />
      <path d="M8 11.4V7.6" />
      <path d="M6.4 9.2 8 7.6l1.6 1.6" />
    </>,
  );
}

export function IconCheck({ className }: IconProps) {
  return frame(className, <path d="M3.4 8.4 6.6 11.5 12.6 4.6" />);
}

export function IconClose({ className }: IconProps) {
  return frame(className, <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" />);
}

export function IconPin({ className, filled }: IconProps & { filled?: boolean }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      <path d="M6 1.8h4l-.6 3.1 2.1 2.1v1H4.5v-1l2.1-2.1z" />
      <path d="M8 8.5V14" />
    </svg>
  );
}

export function IconTrash({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M2.8 4.3h10.4" />
      <path d="M6.2 4.3V3a.9.9 0 0 1 .9-.9h1.8a.9.9 0 0 1 .9.9v1.3" />
      <path d="M4.2 4.3l.6 8.4a1 1 0 0 0 1 .9h4.4a1 1 0 0 0 1-.9l.6-8.4" />
    </>,
  );
}

export function IconCopy({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="5.4" y="5.4" width="8.1" height="8.1" rx="1.4" />
      <path d="M10.6 5.4V3.9a1.4 1.4 0 0 0-1.4-1.4H3.9a1.4 1.4 0 0 0-1.4 1.4v5.3a1.4 1.4 0 0 0 1.4 1.4h1.5" />
    </>,
  );
}

export function IconText({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M2.5 4.2V2.8h11v1.4" />
      <path d="M8 2.8v10.4" />
      <path d="M5.6 13.2h4.8" />
    </>,
  );
}

export function IconImage({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="2.2" y="3.2" width="11.6" height="9.6" rx="1.4" />
      <circle cx="5.9" cy="6.6" r="1.1" />
      <path d="M2.6 11.1 6 8.2l2.3 2 2-1.6 3 2.5" />
    </>,
  );
}

export function IconFiles({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M3 2.6h5l2.6 2.6v8.2H3z" />
      <path d="M8 2.6V5.2h2.6" />
      <path d="M11.4 5.6h1.6v7.8H6.2" />
    </>,
  );
}

export function IconAll({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M2.6 4.4h10.8" />
      <path d="M2.6 8h10.8" />
      <path d="M2.6 11.6h10.8" />
    </>,
  );
}

export function IconPalette({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M8 1.9a6.1 6.1 0 1 0 0 12.2c1.1 0 1.6-.7 1.6-1.5 0-.7-.4-1.1-.4-1.7 0-.6.5-1.1 1.2-1.1h1.3c1.3 0 2.2-.9 2.2-2.2A5.7 5.7 0 0 0 8 1.9z" />
      <circle cx="5.2" cy="6.1" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="8" cy="4.6" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="10.8" cy="6.1" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="5.4" cy="9.4" r="0.9" fill="currentColor" stroke="none" />
    </>,
  );
}

export function IconTag({ className }: IconProps) {  return frame(
    className,
    <>
      <path d="M7.4 2.4H13v5.6l-5.9 5.9a1.2 1.2 0 0 1-1.7 0L2.5 11a1.2 1.2 0 0 1 0-1.7z" />
      <circle cx="10.4" cy="5.2" r="0.9" />
    </>,
  );
}

export function IconFolder({ className }: IconProps) {
  return frame(
    className,
    <path d="M2.2 4.1a1.2 1.2 0 0 1 1.2-1.2h2.4l1.4 1.7h5.4a1.2 1.2 0 0 1 1.2 1.2v5.7a1.2 1.2 0 0 1-1.2 1.2H3.4a1.2 1.2 0 0 1-1.2-1.2z" />,
  );
}

export function IconPlus({ className }: IconProps) {
  return frame(className, <path d="M8 3.4v9.2M3.4 8h9.2" />);
}

export function IconMinus({ className }: IconProps) {
  return frame(className, <path d="M3.4 8h9.2" />);
}

export function IconLoop({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M3 6.6a3.4 3.4 0 0 1 3.4-3.4h6.2" />
      <path d="M10.8 1.8 12.9 3.2 10.8 4.6" />
      <path d="M13 9.4a3.4 3.4 0 0 1-3.4 3.4H3.4" />
      <path d="M5.2 11.4 3.1 12.8 5.2 14.2" />
    </>,
  );
}

export function IconShield({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M8 1.9 13 4v4c0 3-2.1 5.2-5 6.1C5.1 13.2 3 11 3 8V4z" />
      <path d="M6.1 8.1l1.4 1.4 2.6-2.7" />
    </>,
  );
}

export function IconChevronLeft({ className }: IconProps) {
  return frame(className, <path d="M9.8 3.4 5.2 8l4.6 4.6" />);
}

export function IconChevronRight({ className }: IconProps) {
  return frame(className, <path d="M6.2 3.4 10.8 8l-4.6 4.6" />);
}

export function IconMinimize({ className }: IconProps) {
  return frame(className, <path d="M3.5 8.5h9" />);
}

export function IconMaximize({ className }: IconProps) {
  return frame(className, <rect x="3.5" y="3.5" width="9" height="9" rx="1.2" />);
}

export function IconRestore({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="3" y="5.6" width="7.4" height="7.4" rx="1.1" />
      <path d="M5.8 5.6V4.2a1.2 1.2 0 0 1 1.2-1.2h5.2a1.2 1.2 0 0 1 1.2 1.2v5.2a1.2 1.2 0 0 1-1.2 1.2h-1.4" />
    </>,
  );
}

export function IconList({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M5.6 4.2h7.9M5.6 8h7.9M5.6 11.8h7.9" />
      <path d="M2.9 4.2h.01M2.9 8h.01M2.9 11.8h.01" strokeWidth={2} />
    </>,
  );
}

export function IconGrid({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="2.6" y="2.6" width="4.6" height="4.6" rx="1" />
      <rect x="8.8" y="2.6" width="4.6" height="4.6" rx="1" />
      <rect x="2.6" y="8.8" width="4.6" height="4.6" rx="1" />
      <rect x="8.8" y="8.8" width="4.6" height="4.6" rx="1" />
    </>,
  );
}

export function IconSave({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M3 2.6h7.2L13 5.4v8a.9.9 0 0 1-.9.9H3.9a.9.9 0 0 1-.9-.9V3.5a.9.9 0 0 1 .9-.9z" />
      <path d="M5.4 2.6v3.2h4.3V2.6" />
      <path d="M4.8 14.3V9.1h6.4v5.2" />
    </>,
  );
}

export function IconExternal({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M9.2 2.6h4.2v4.2" />
      <path d="M13.4 2.6 7.4 8.6" />
      <path d="M11.6 9.6v3a1 1 0 0 1-1 1h-6a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1h3" />
    </>,
  );
}

export function IconMail({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="2" y="3.4" width="12" height="9.2" rx="1.4" />
      <path d="m2.6 4.4 5.4 4 5.4-4" />
    </>,
  );
}

export function IconPause({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M5.8 3.4v9.2M10.2 3.4v9.2" strokeWidth={1.9} />
    </>,
  );
}

export function IconPlay({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M5.2 3.2 12 8l-6.8 4.8z" />
    </>,
  );
}

export function IconClock({ className }: IconProps) {
  return frame(
    className,
    <>
      <circle cx="8" cy="8" r="5.6" />
      <path d="M8 4.9V8l2.2 1.6" />
    </>,
  );
}

export function IconStar({ className, filled }: IconProps & { filled?: boolean }) {
  return frame(
    className,
    <>
      <path
        d="M8 2.4l1.75 3.6 3.95.55-2.87 2.8.69 3.95L8 11.42 4.48 13.3l.69-3.95L2.3 6.55l3.95-.55z"
        fill={filled ? "currentColor" : "none"}
      />
    </>,
  );
}

export function IconEmoji({ className }: IconProps) {
  return frame(
    className,
    <>
      <circle cx="8" cy="8" r="5.8" />
      <path d="M5.6 9.3a3.1 3.1 0 0 0 4.8 0" />
      <path d="M5.9 6.1h.01M10.1 6.1h.01" strokeWidth={2} />
    </>,
  );
}

export function IconGif({ className }: IconProps) {
  return frame(
    className,
    <>
      <rect x="1.6" y="3.4" width="12.8" height="9.2" rx="1.6" />
      <path d="M6.3 8.9 9.5 8l-3.2 1.6z" fill="currentColor" />
      <path d="M3.6 10.4h1.2M11.2 10.4h1.2" />
    </>,
  );
}

export function IconEyedropper({ className }: IconProps) {
  return frame(
    className,
    <path d="M11.33 6.31L4.4 13.23A1.16 1.16 0 0 1 2.77 11.6L9.69 4.67M11.56 6.54L13.23 4.87A0.5 0.5 0 0 1 13.23 4.17L11.83 2.77A0.5 0.5 0 0 1 11.13 2.77L9.46 4.44M11.56 6.54L9.46 4.44" />,
  );
}

export function IconNote({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M3.2 2.6h6.2l3.4 3.4v7.4a.8.8 0 0 1-.8.8H3.2a.8.8 0 0 1-.8-.8V3.4a.8.8 0 0 1 .8-.8z" />
      <path d="M9.2 2.8v3.2h3.2M5 8.2h5.4M5 10.6h3.8" />
    </>,
  );
}


