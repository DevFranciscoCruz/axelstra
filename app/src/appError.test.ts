import { describe, expect, it } from "vitest";
import { AppError, errorText, toAppError } from "./appError";
import { norm } from "./paths";

describe("norm", () => {
  it("makes both slash styles, case and trailing slashes equal", () => {
    expect(norm("C:\\Work\\Proj\\")).toBe("c:/work/proj");
    expect(norm("C:/work/proj//")).toBe("c:/work/proj");
    expect(norm(null)).toBe("");
  });
});

describe("errorText", () => {
  it("never gives [object Object]", () => {
    expect(errorText({ message: "boom" })).toBe("boom");
    expect(errorText({ code: 3 })).toBe('{"code":3}');
    expect(errorText({})).toBe("Erro desconhecido");
    expect(errorText(undefined)).toBe("Erro desconhecido");
    expect(errorText("plain")).toBe("plain");
    expect(errorText(new Error("e"))).toBe("e");
  });
});

describe("AppError", () => {
  it("keeps the method and reads as its message in String()", () => {
    const e = toAppError("git.push", { error: "rejected" });
    expect(e).toBeInstanceOf(AppError);
    expect(e.method).toBe("git.push");
    expect(String(e)).toBe("rejected");
    expect(toAppError("x", e)).toBe(e);
  });
});

describe("cancelled request", () => {
  it("is recognised and never shown as a failure", async () => {
    const { CANCELLED, isCancelled } = await import("./appError");
    const { setError } = await import("./store/notify");
    const { toasts } = await import("./toasts");
    const e = new AppError("git.log", "Request cancelled", CANCELLED);
    expect(isCancelled(e)).toBe(true);
    expect(isCancelled(new AppError("git.log", "x"))).toBe(false);
    setError(errorText(e));
    expect(toasts()).toHaveLength(0);
  });
});
