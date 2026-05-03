export const $ = <T extends HTMLElement>(selector: string): T => {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`Missing element: ${selector}`);
  }
  return element;
};

export type ConfirmOptions = {
  title?: string;
  okLabel?: string;
  cancelLabel?: string;
};

export function confirmDialog(message: string, options: ConfirmOptions = {}): Promise<boolean> {
  const modal = $<HTMLDivElement>("#confirmModal");
  const titleEl = $<HTMLHeadingElement>("#confirmTitle");
  const messageEl = $<HTMLParagraphElement>("#confirmMessage");
  const okBtn = $<HTMLButtonElement>("#confirmOk");
  const cancelBtn = $<HTMLButtonElement>("#confirmCancel");

  titleEl.textContent = options.title ?? "Bestätigen";
  messageEl.textContent = message;
  okBtn.textContent = options.okLabel ?? "OK";
  cancelBtn.textContent = options.cancelLabel ?? "Abbrechen";

  return new Promise<boolean>((resolve) => {
    const cleanup = (result: boolean) => {
      modal.hidden = true;
      okBtn.removeEventListener("click", onOk);
      cancelBtn.removeEventListener("click", onCancel);
      modal.removeEventListener("click", onBackdrop);
      document.removeEventListener("keydown", onKey);
      resolve(result);
    };
    const onOk = () => cleanup(true);
    const onCancel = () => cleanup(false);
    const onBackdrop = (e: MouseEvent) => {
      if (e.target === modal) cleanup(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cleanup(false);
      else if (e.key === "Enter") cleanup(true);
    };

    okBtn.addEventListener("click", onOk);
    cancelBtn.addEventListener("click", onCancel);
    modal.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKey);

    modal.hidden = false;
    queueMicrotask(() => okBtn.focus());
  });
}

export function showModal(selector: string) {
  $(selector).hidden = false;
}

export function hideModal(selector: string) {
  $(selector).hidden = true;
}

export function escapeHtml(value: unknown): string {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
