import type { KeyboardEvent } from "react";

/** Shared shortcuts for the initial prompt and follow-up composer. */
export function handlePromptKeyDown(
  event: KeyboardEvent<HTMLTextAreaElement>,
  onSubmit: () => void,
) {
  if (event.nativeEvent.isComposing) return;

  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    onSubmit();
  }
}
