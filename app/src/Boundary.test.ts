import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createComponent, createRenderEffect, createResource, createSignal } from "solid-js";

vi.mock("./api", async () => {
  const { createLogoLoader } = await import("./logo");
  return {
    copyDiagnostics: vi.fn(async () => undefined),
    logFrontendError: vi.fn(async () => undefined),
    // the real shape: the daemon call fails, the loader turns it into "no logo"
    projectLogo: createLogoLoader(() =>
      Promise.reject(new Error("Sem resposta do daemon (project.logo) ao fim de 6 s.")),
    ),
  };
});

import { Fallback, Section } from "./Boundary";
import { LOCALES, setLocale, type Locale } from "./i18n";
import { S } from "./strings";
import { U } from "./strings.ui";
import { AppError } from "./appError";
import { ProjectIcon } from "./sidebar/ProjectIcon";

afterEach(() => {
  cleanup();
  setLocale("pt-PT");
  document.body.innerHTML = "";
});

const mountFallback = (props: Partial<Parameters<typeof Fallback>[0]> = {}) =>
  render(() => createComponent(Fallback, { name: "projetos", error: new Error("kaput"), reset: () => {}, ...props }));

describe("ErrorBoundary fallback", () => {
  it("is an alert with title, detail and both actions, focus on retry", () => {
    const r = mountFallback();
    const alert = r.getByRole("alert");
    expect(alert.querySelector("h2")?.textContent).toBe(U.boundary.part("projetos"));
    expect(alert.textContent).toContain("kaput");
    const retry = r.getByText(S.retry);
    expect(document.activeElement).toBe(retry);
    expect(r.getByText(U.boundary.copyDiag)).toBeTruthy();
    expect(alert.querySelector("[style]")).toBeNull();
  });

  it("whole offers a reload", () => {
    const r = mountFallback({ whole: true });
    expect(r.getByText(U.boundary.reload)).toBeTruthy();
    expect(r.getByRole("alert").classList.contains("whole")).toBe(true);
  });

  it.each(LOCALES as readonly Locale[])("reads in %s", (loc) => {
    setLocale(loc);
    const r = mountFallback();
    expect(r.getByText(S.retry)).toBeTruthy();
    expect(r.getByText(U.boundary.copyDiag)).toBeTruthy();
    expect(r.getByRole("alert").querySelector("h2")?.textContent).toBe(U.boundary.part("projetos"));
  });

  it("retry remounts the child and fetches its resource again", async () => {
    let fetches = 0;
    let mounts = 0;
    const [fail, setFail] = createSignal(true);
    const Child = () => {
      mounts++;
      const [data] = createResource(async () => {
        fetches++;
        if (fail()) throw new AppError("projects", "Sem resposta do daemon (project.logo) ao fim de 30 s.");
        return "ok";
      });
      // reading the failed resource throws into the boundary, as a component would
      const el = document.createElement("span");
      createRenderEffect(() => (el.textContent = data() ?? ""));
      return el;
    };
    const r = render(() =>
      createComponent(Section, {
        name: "projetos",
        get children() {
          return createComponent(Child, {});
        },
      }),
    );
    await waitFor(() => expect(r.getByRole("alert")).toBeTruthy());
    expect(fetches).toBe(1);
    setFail(false);
    fireEvent.click(r.getByText(S.retry));
    await waitFor(() => expect(r.queryByRole("alert")).toBeNull());
    expect(mounts).toBe(2);
    expect(fetches).toBe(2);
    expect(r.container.textContent).toContain("ok");
  });

  it("a failing project logo never reaches the boundary: letter icon stays", async () => {
    const r = render(() =>
      createComponent(Section, {
        name: "x",
        get children() {
          return createComponent(ProjectIcon, { id: "p1", name: "Proj" });
        },
      }),
    );
    await waitFor(() => expect(r.container.querySelector(".picon")).toBeTruthy());
    expect(r.queryByRole("alert")).toBeNull();
  });
});
