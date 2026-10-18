import type { ClipAction } from "../lib/actions";
import { IconExternal, IconFolder } from "./icons";

interface ClipActionButtonProps {
  action: ClipAction;
  onRun: (action: ClipAction) => void;
  className?: string;
  iconClassName?: string;
}

/** Inline button for a clip type's custom action, e.g. "Open Folder". */
export function ClipActionButton({
  action,
  onRun,
  className,
  iconClassName = "size-3.5",
}: ClipActionButtonProps) {
  const Icon = action.kind === "open-url" ? IconExternal : IconFolder;
  return (
    <button
      type="button"
      title={action.label}
      aria-label={action.label}
      onClick={(event) => {
        event.stopPropagation();
        onRun(action);
      }}
      className={className}
    >
      <Icon className={iconClassName} />
    </button>
  );
}
