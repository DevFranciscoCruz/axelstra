import { describe, expect, it, vi } from "vitest";
import { LOGO_TRIES, createLogoLoader, type LogoAnswer } from "./logo";
import { COSMETIC_TIMEOUT_MS, timeoutFor } from "./api";

const URL = "data:image/png;base64,AAAA";

describe("project logo fallback", () => {
  it("returns the data url when the daemon has one", async () => {
    const load = createLogoLoader(async () => ({ data_url: URL }));
    expect(await load("p1")).toBe(URL);
  });

  it.each<[string, () => Promise<LogoAnswer>]>([
    ["timeout", () => Promise.reject(new Error("Sem resposta do daemon (project.logo) ao fim de 6 s."))],
    ["error", () => Promise.reject("boom")],
    ["null answer", async () => null],
    ["undefined answer", async () => undefined],
    ["null data_url", async () => ({ data_url: null })],
    ["empty data_url", async () => ({ data_url: "" })],
    ["loading", async () => ({ loading: true })],
  ])("%s -> null, never throws", async (_n, send) => {
    const load = createLogoLoader(send);
    await expect(load("p1")).resolves.toBeNull();
  });

  it("remembers a found logo and a plain miss for the session", async () => {
    const send = vi.fn(async () => ({ data_url: null }));
    const load = createLogoLoader(send);
    await load("p1");
    await load("p1");
    expect(send).toHaveBeenCalledTimes(1);
  });

  it("retries a failure once, then keeps the fallback (no loop)", async () => {
    const send = vi.fn(async () => Promise.reject(new Error("late")));
    const load = createLogoLoader(send);
    for (let i = 0; i < 5; i++) await load("p1");
    expect(send).toHaveBeenCalledTimes(LOGO_TRIES);
  });

  it("a loading answer can become a logo on the retry", async () => {
    const answers: LogoAnswer[] = [{ loading: true }, { data_url: URL }];
    const load = createLogoLoader(async () => answers.shift());
    expect(await load("p1")).toBeNull();
    expect(await load("p1")).toBe(URL);
  });

  it("joins a look already running", async () => {
    const send = vi.fn(async () => ({ data_url: URL }));
    const load = createLogoLoader(send);
    await Promise.all([load("p1"), load("p1")]);
    expect(send).toHaveBeenCalledTimes(1);
  });

  it("project.logo has a short client timeout", () => {
    expect(timeoutFor("project.logo")).toBe(COSMETIC_TIMEOUT_MS);
    expect(COSMETIC_TIMEOUT_MS).toBeLessThanOrEqual(6_000);
    expect(timeoutFor("project.list")).toBeGreaterThan(COSMETIC_TIMEOUT_MS);
  });
});
