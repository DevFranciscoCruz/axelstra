import { describe, expect, it } from "vitest";
import { downloadErrorText, micErrorText, transcribeErrorText } from "./voiceErrors";

describe("voice error texts (U19)", () => {
  it("explains a refused, missing and busy microphone", () => {
    expect(micErrorText({ name: "NotAllowedError" })).toMatch(/recusado/);
    expect(micErrorText(new DOMException("x", "NotFoundError"))).toMatch(/nenhum microfone/);
    expect(micErrorText({ name: "NotReadableError" })).toMatch(/outra aplicação/);
  });
  it("never leaks the raw exception", () => {
    expect(micErrorText("TypeError: boom at x.js:1")).not.toMatch(/boom|TypeError/);
    expect(transcribeErrorText("Error: sherpa panic at lib.rs:99")).not.toMatch(/sherpa|panic/);
    expect(downloadErrorText("ECONNRESET")).not.toMatch(/ECONNRESET/);
  });
});
