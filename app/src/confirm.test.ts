import { afterEach, describe, expect, it } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import { ConfirmHost, ask, focusesCancel } from "./Confirm";

describe("confirm", () => {
  let dispose = () => {};
  afterEach(() => {
    dispose();
    document.body.innerHTML = "";
  });
  const focused = async (kind: "warning" | "info" | "error" | undefined) => {
    dispose = render(() => createComponent(ConfirmHost, {}), document.body);
    void ask("Sem volta?", { kind, okLabel: "Apagar", cancelLabel: "Manter" });
    await new Promise((r) => setTimeout(r, 0));
    return (document.activeElement as HTMLElement | null)?.textContent;
  };

  it("destructive kinds focus the safe button", () => {
    expect(focusesCancel("warning")).toBe(true);
    expect(focusesCancel("error")).toBe(true);
    expect(focusesCancel("info")).toBe(false);
    expect(focusesCancel(undefined)).toBe(false);
  });
  it("a warning opens with Cancel focused and the confirm button marked", async () => {
    expect(await focused("warning")).toBe("Manter");
    expect(document.querySelector(".primary.kind-warn")?.textContent).toBe("Apagar");
  });
  it("an info question opens with the confirm button focused", async () => {
    expect(await focused("info")).toBe("Apagar");
  });
});
