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
      <circle cx="8" cy="8" r="2.2" />
      <path d="M8 1.5v1.8M8 12.7v1.8M14.5 8h-1.8M3.3 8H1.5M12.6 3.4l-1.3 1.3M4.7 11.3l-1.3 1.3M12.6 12.6l-1.3-1.3M4.7 4.7 3.4 3.4" />
    </>,
  );
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

export function IconTag({ className }: IconProps) {
  return frame(
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

export function IconShield({ className }: IconProps) {
  return frame(
    className,
    <>
      <path d="M8 1.9 13 4v4c0 3-2.1 5.2-5 6.1C5.1 13.2 3 11 3 8V4z" />
      <path d="M6.1 8.1l1.4 1.4 2.6-2.7" />
    </>,
  );
}
