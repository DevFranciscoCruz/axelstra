import { describe, expect, it } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import * as ui from "./index";
import { Gallery } from "./Gallery";

const noop = () => {};
/** props that let each component render on its own; a new export fails the test until it is listed here */
const props: Record<string, Record<string, unknown>> = {
  Button: { children: "x" },
  Badge: { children: "x" },
  Input: {},
  Textarea: {},
  Field: { label: "l", children: "x" },
  Checkbox: { label: "l" },
  CheckboxInput: {},
  CheckboxField: { checked: false, onChange: noop, children: "x" },
  RadioGroup: { label: "l", value: "a", onChange: noop, options: [{ value: "a", label: "A" }, { value: "b", label: "B" }] },
  Select: { value: "a", onChange: noop, options: [{ value: "a", label: "A" }] },
  Dialog: { label: "d", onClose: noop, children: "x" },
  Tabs: { value: "a", onChange: noop, children: "x" },
  DropdownMenu: { open: false, onOpenChange: noop, x: 0, y: 0, items: [{ label: "a", run: noop }] },
};
const helpers = ["buttonVariants", "badgeVariants", "cn"];
/** pieces that only render inside their parent (Tabs) */
const parts = ["TabsList", "TabsTrigger"];

describe("ui/index", () => {
  it("renders every exported component without throwing", () => {
    for (const [name, value] of Object.entries(ui)) {
      if (helpers.includes(name)) {
        expect(typeof value, name).toBe("function");
        continue;
      }
      if (parts.includes(name)) continue;
      expect(props, `${name} needs sample props`).toHaveProperty(name);
      const host = document.createElement("div");
      document.body.append(host);
      const dispose = render(() => createComponent(value as never, props[name] as never), host);
      expect(document.body.innerHTML.length, name).toBeGreaterThan(0);
      dispose();
      host.remove();
    }
  });

  it("renders the Gallery", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => createComponent(Gallery, {}), host);
    expect(host.querySelectorAll('[role="radiogroup"]')).toHaveLength(1);
    dispose();
    host.remove();
  });

  it("RadioGroup keeps roving radios and arrow / Home / End selection", () => {
    const seen: string[] = [];
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => createComponent(ui.RadioGroup, { ...props.RadioGroup, onChange: (v: string) => seen.push(v) } as never), host);
    const group = host.querySelector("[role=radiogroup]") as HTMLElement;
    const radios = [...group.querySelectorAll<HTMLInputElement>("input[type=radio]")];
    expect(radios).toHaveLength(2);
    radios[0].focus();
    const press = (key: string) => group.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    press("ArrowRight");
    expect(document.activeElement).toBe(radios[1]);
    press("ArrowRight");
    expect(document.activeElement).toBe(radios[0]);
    press("End");
    expect(document.activeElement).toBe(radios[1]);
    press("Home");
    expect(document.activeElement).toBe(radios[0]);
    expect(seen).toContain("b");
    dispose();
    host.remove();
  });
});
