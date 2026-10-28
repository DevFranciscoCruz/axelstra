import { afterEach, describe, expect, it } from "vitest";
import type { ScanxFolder, ScanxStatus } from "./api";
import { setLocale } from "./i18n";
import {
  buttons,
  defenderLines,
  folderRows,
  hintVisible,
  jobMessage,
  platformNote,
  procLine,
  statusError,
} from "./scanxLogic";
import { PF } from "./strings.perf";

afterEach(() => setLocale("pt-PT"));

const folder = (o: Partial<ScanxFolder> = {}): ScanxFolder => ({
  project: "p1",
  name: "proj",
  path: "C:\\x\\.axelstra-wt",
  exists: true,
  indexable: false,
  excluded: "unknown",
  source: null,
  drive: "C",
  fs: "NTFS",
  dev_drive: false,
  ...o,
});
const status = (o: Partial<ScanxStatus> = {}): ScanxStatus => ({
  os: "windows",
  defender: { state: "active", realtime: true, tamper: false, exclusions_readable: false },
  folders: [folder()],
  can_apply: true,
  can_revert: false,
  hint: true,
  hint_dismissed: false,
  job: { running: null, last: null },
  ...o,
});

describe("hintVisible", () => {
  it("shows only while pending, not dismissed, on Windows with Defender active", () => {
    expect(hintVisible(status())).toBe(true);
    expect(hintVisible(status({ hint: false }))).toBe(false);
    expect(hintVisible(status({ hint_dismissed: true }))).toBe(false);
    expect(hintVisible(status({ os: "linux" }))).toBe(false);
    expect(
      hintVisible(
        status({ defender: { state: "inactive", realtime: null, tamper: null, exclusions_readable: false } }),
      ),
    ).toBe(false);
    expect(hintVisible(null)).toBe(false);
  });
});

describe("folderRows", () => {
  it("merges projects that share a folder and labels each state", () => {
    const rows = folderRows(
      status({
        folders: [
          folder(),
          folder({ project: "p2", name: "other" }),
          folder({ path: "D:\\w", excluded: "yes", source: "recorded" }),
        ],
      }),
    );
    expect(rows).toHaveLength(2);
    expect(rows[0].names).toEqual(["proj", "other"]);
    expect(rows[0].indexable).toBe(PF.indexable.no);
    expect(rows[0].excluded).toBe(PF.excluded.unknown);
    expect(rows[0].fs).toBe("C: NTFS");
    expect(rows[1].excluded).toBe(PF.excluded.recorded);
  });
  it("marks a folder that does not exist and an unknown index (Linux)", () => {
    const [r] = folderRows(
      status({
        os: "linux",
        folders: [folder({ exists: false, indexable: null, excluded: "n/a", fs: null, drive: null })],
      }),
    );
    expect(r.missing).toBe(true);
    expect(r.indexable).toBe(PF.indexable.unknown);
    expect(r.excluded).toBe(PF.excluded.na);
    expect(r.fs).toBe("");
  });
  it("follows the language", () => {
    setLocale("en");
    expect(folderRows(status())[0].indexable).toBe("kept out of the system index");
  });
});

describe("defenderLines / platformNote", () => {
  it("explains tamper protection and unreadable exclusions", () => {
    const s = status({ defender: { state: "active", realtime: true, tamper: true, exclusions_readable: false } });
    expect(defenderLines(s)).toEqual([PF.defenderState.active, PF.realtimeOn, PF.tamperOn, PF.unreadable]);
  });
  it("says nothing about Defender off Windows and gives a neutral note", () => {
    const s = status({
      os: "macos",
      defender: { state: "n/a", realtime: null, tamper: null, exclusions_readable: false },
    });
    expect(defenderLines(s)).toEqual([]);
    expect(platformNote(s)).toBe(PF.macos);
    expect(platformNote(status({ os: "linux" }))).toBe(PF.linux);
    expect(platformNote(status())).toBeNull();
  });
});

describe("buttons", () => {
  it("apply needs the consent and a free daemon; never enabled by default", () => {
    expect(buttons(status(), false, false).applyDisabled).toBe(true);
    expect(buttons(status(), true, false).applyDisabled).toBe(false);
    expect(buttons(status(), true, true).applyDisabled).toBe(true);
    expect(buttons(status({ job: { running: "apply", last: null } }), true, false).applyDisabled).toBe(true);
    expect(buttons(status({ can_apply: false }), true, false).applyDisabled).toBe(true);
  });
  it("revert only shows when something was applied", () => {
    expect(buttons(status(), false, false).showRevert).toBe(false);
    expect(buttons(status({ can_revert: true }), false, false).showRevert).toBe(true);
    expect(buttons(status({ can_revert: true }), false, true).revertDisabled).toBe(true);
  });
  it("offers no controls off Windows or without folders", () => {
    expect(buttons(status({ os: "linux" }), true, false).showControls).toBe(false);
    expect(buttons(status({ folders: [] }), true, false).showControls).toBe(false);
    expect(buttons(status(), true, false).showControls).toBe(true);
  });
});

describe("jobMessage", () => {
  it("reads results and coded errors in the UI language", () => {
    expect(jobMessage(null)).toBeNull();
    expect(jobMessage({ op: "hint", ok: true })).toBeNull();
    expect(jobMessage({ op: "apply", ok: true, changed: ["a"], unchanged: [] })).toEqual({
      kind: "ok",
      text: PF.applied(1, 0),
    });
    expect(jobMessage({ op: "apply", ok: true, changed: [], unchanged: ["a"] })?.text).toBe(PF.appliedNone);
    expect(jobMessage({ op: "revert", ok: true, changed: ["a", "b"] })?.text).toBe(PF.reverted(2));
    const e = jobMessage({
      op: "revert",
      ok: false,
      error: "English",
      error_code: "scanx.elevation_cancelled",
      error_params: {},
    });
    expect(e?.kind).toBe("err");
    expect(e?.text).toContain("pedido de administrador");
    setLocale("en");
    expect(
      jobMessage({ op: "revert", ok: false, error: "x", error_code: "scanx.elevation_cancelled" })?.text,
    ).toContain("administrator prompt");
  });
  it("falls back to the English text of an unknown code", () => {
    expect(jobMessage({ op: "apply", ok: false, error: "boom", error_code: "scanx.nope" })?.text).toBe(
      PF.failed("boom"),
    );
  });
});

describe("statusError / procLine", () => {
  it("only reports an error when the query failed", () => {
    expect(statusError(status())).toBeNull();
    expect(
      statusError(status({ error: "x", error_code: "scanx.powershell_failed", error_params: { detail: "d" } })),
    ).toContain("d");
  });
  it("formats cpu and memory, with a dash for a missing reading", () => {
    expect(procLine({ name: "MsMpEng", pid: 4, cpu: 64, rss: 685256704 })).toContain("MsMpEng: 64 % ");
    expect(procLine({ name: "SearchIndexer", pid: 5, cpu: null, rss: null })).toBe("SearchIndexer: — · —");
  });
});
