import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";

// The temp dir in its canonical form. On the Windows runners os.tmpdir() is the 8.3 short name (C:\Users\RUNNER~1\...)
// while git and the daemon register the long one (C:\Users\runneradmin\...), so a path built from the short form is
// "not a worktree of a registered project". Every driver builds its repos, worktrees and data dirs from this.
export const realTmpdir = () => realpathSync.native(tmpdir());
