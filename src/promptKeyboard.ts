import type { KeyboardEvent } from "react";

/** Shared shortcuts for the initial prompt and follow-up composer. */
export function handlePromptKeyDown(
  event: KeyboardEvent<HTMLTextAreaElement>,
  onChange: (value: string) => void,
  onSubmit: () => void,
) {
  if (event.nativeEvent.isComposing) return;

  if (event.key === " " && event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey) {
    event.preventDefault();
    const input = event.currentTarget;
    const cursor = input.selectionStart + 1;
    input.setRangeText("\n", input.selectionStart, input.selectionEnd, "end");
    onChange(input.value);
    requestAnimationFrame(() => {
      if (input.isConnected) input.setSelectionRange(cursor, cursor);
    });
  } else if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    onSubmit();
  }
}
