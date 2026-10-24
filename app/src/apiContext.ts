import type { Project, Terse } from "./api";

/** What the store tells the api layer about projects and reply styles, so api.ts does not import the store (they used to import each other). */
export const apiContext: { projects: () => Project[]; terse: (project?: string | null) => Terse | undefined } = {
  projects: () => [],
  terse: () => undefined,
};
