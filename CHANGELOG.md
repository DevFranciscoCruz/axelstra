# Changelog

## [0.2.0](https://github.com/DevFranciscoCruz/axelstra/compare/v0.1.42...v0.2.0) (2026-10-31)


### Features

* **app:** describe the stall reminder and the 15 min default in settings and docs ([da4db4a](https://github.com/DevFranciscoCruz/axelstra/commit/da4db4a00c024d1aaae5b32db42608d159ecd775))
* **app:** hibernation UI (dot, menu, settings, notices; pt-PT/en/pt-BR), docs for spawn queue and governor ([0bf2dd4](https://github.com/DevFranciscoCruz/axelstra/commit/0bf2dd46f3912dbc461d0e359eb6c1153c580cf0))
* **app:** local explorer commands (list/stat/read/open/reveal/watch) confined to registered roots, served without the daemon ([2677772](https://github.com/DevFranciscoCruz/axelstra/commit/26777727d0910570b12c77e0efa082be9e577e23))
* **app:** notify the human only when needed: categories, do-not-disturb, aggregation; no alert for stall_parent or a quiet agent ([550b753](https://github.com/DevFranciscoCruz/axelstra/commit/550b7537092263de0cb94346cea91d2373181bd9))
* **app:** replace a stale daemon (other commit) when idle, else offer a restart ([8276299](https://github.com/DevFranciscoCruz/axelstra/commit/8276299b55789ba39739993f8880ee7af0783b64))
* **app:** resource governor part 3: memory slice, status bar chip and panel, paused state in sidebar, Settings section, i18n ([598c2ca](https://github.com/DevFranciscoCruz/axelstra/commit/598c2ca2f5e5b9a5427590e5a4462bc4b37a07b5))
* **app:** settings as a full page with grouped side navigation, search, Esc/back; remove modal ([6425b0a](https://github.com/DevFranciscoCruz/axelstra/commit/6425b0a4e199e78865951cd4dd0d23a8d12a59d2))
* **app:** Settings section for the build cost (jobs, sccache, shared target, JS cache, lock budget and margin) ([74cf0c6](https://github.com/DevFranciscoCruz/axelstra/commit/74cf0c6ba0bd2f2845a3feaa197b3bf30dd71dfa))
* **app:** slow daemon calls show a cancellable notice instead of failing by elapsed time ([c3af63f](https://github.com/DevFranciscoCruz/axelstra/commit/c3af63f12d5aace07aaab7bc63ca7106941e1925))
* **app:** version-skew notice shows kept-old daemon (daemon_offer) with app and daemon builds ([84a39f0](https://github.com/DevFranciscoCruz/axelstra/commit/84a39f03a8781383e8b7aa69fed3b66f928afd3b))
* **app:** worktree sizes, Limpar builds, idle unintegrated-work warning, disk settings ([afcea3b](https://github.com/DevFranciscoCruz/axelstra/commit/afcea3bba86b4a7ad3783c9a41f5d4541040dd66))
* **cli:** axelstra disk, worktree sweep --artifacts, disk.ensure before axelstra lock ([fdf1a5e](https://github.com/DevFranciscoCruz/axelstra/commit/fdf1a5e66bfcbde0ba9f51e54bf9437c36787331))
* **cli:** axelstra lock --mem-mb memory-budget admission (event-driven, build/verify default from jobs) ([74cfad8](https://github.com/DevFranciscoCruz/axelstra/commit/74cfad884eb907a3cb083f43c3c94bf99830c4f3))
* **core:** pure ps parsers, Windows rss in the process table, shared name cleaner ([1409f40](https://github.com/DevFranciscoCruz/axelstra/commit/1409f40dde4b183fdb3628fbed6f8e4dbc0e6b67))
* **daemon:** buildenv - derived CARGO_BUILD_JOBS, optional sccache, shared caches merged into every spawned session ([a459a33](https://github.com/DevFranciscoCruz/axelstra/commit/a459a33c2a3b998453b83e115ef75e6f36406722))
* **daemon:** count child processes per site (method or loop) and command in breakdown.own.sites ([6dd0b1d](https://github.com/DevFranciscoCruz/axelstra/commit/6dd0b1db50c042dd31ff080f530d3cc1c79b28a1))
* **daemon:** Defender exclusion command in the FMfn kernel pool notice (Windows) ([16a20a5](https://github.com/DevFranciscoCruz/axelstra/commit/16a20a5cf34c9741b17fb47321dbfe166bd6528c))
* **daemon:** janitor reports a held integrated worktree once (kind pending, holder name/pid); tests for squash, cherry-pick and orch/* integration ([dc52cd9](https://github.com/DevFranciscoCruz/axelstra/commit/dc52cd9282837ff4bcf1fc50176c56bcf596cdd2))
* **daemon:** lead per request, not per worktree; stall watchdog ignores a lead that never got a request ([d9778ca](https://github.com/DevFranciscoCruz/axelstra/commit/d9778ca39fb881e8728fb4799f85b3bf8076badc))
* **daemon:** let a coordinator pull session state and block until a report arrives ([935e379](https://github.com/DevFranciscoCruz/axelstra/commit/935e379d9363919025e4856fca7fd9d9699458a8))
* **daemon:** list listening ports from /proc on Linux, lsof as fallback ([0a93b67](https://github.com/DevFranciscoCruz/axelstra/commit/0a93b67be3ecce2fcdb932e5c15fbe752e53e682))
* **daemon:** macOS memory for build jobs and microsecond process start times ([8aa82d9](https://github.com/DevFranciscoCruz/axelstra/commit/8aa82d9d971cbc66650ea2fdd49a4b1c873e8de2))
* **daemon:** macOS memory source (host_statistics64) and breakdown.unavailable reasons ([3221746](https://github.com/DevFranciscoCruz/axelstra/commit/32217468ef90952276d4bf6d49e7c9cec522eb57))
* **daemon:** main checkout refuses tasks (session.main_no_tasks) ([7225ca5](https://github.com/DevFranciscoCruz/axelstra/commit/7225ca53a03d6356971c4a25180342a022b3b2e6))
* **daemon:** mark every session process with AXELSTRA_MARK and sweep marked leftovers on teardown (Linux) ([30196f4](https://github.com/DevFranciscoCruz/axelstra/commit/30196f4cf81edbf7b8eca1d0b3335f6027bdefc2))
* **daemon:** reclaim build artifacts of inactive worktrees, disk.status/disk.ensure, PROTO 33 ([c93633e](https://github.com/DevFranciscoCruz/axelstra/commit/c93633e8a6a9032de541f16eb32d22cc65703331))
* **daemon:** remind an agent once when its turn ends silently and tell the parent only of a real stall ([5dff4af](https://github.com/DevFranciscoCruz/axelstra/commit/5dff4af012d24dda7257cf3d6c214edba7073517))
* **daemon:** resource governor part 2: admission, pause/resume, mem.pause/resume/admit/hold, axelstra mem/resume, lock waits for memory ([e15533c](https://github.com/DevFranciscoCruz/axelstra/commit/e15533c4f5c5f32b196182b0de8c6d92c695f4fb))
* **daemon:** session hibernation tests, worker flag kept in meta, rows for sessions between sleep and process, AGENTS.md section ([903b821](https://github.com/DevFranciscoCruz/axelstra/commit/903b821294215a038743fbce3b3ab545f6557c51))
* **daemon:** settings.get reports and settings.set stores the validated build object ([e4975b1](https://github.com/DevFranciscoCruz/axelstra/commit/e4975b1c6edd2d5b733045a668cc2cee4b3f6e4d))
* **governor:** kernel pool tags in mem.status/axelstra mem/panel/notice; own spawns and watches ([c7fd4f1](https://github.com/DevFranciscoCruz/axelstra/commit/c7fd4f128d98a1cadaa3369536ff04fdf2eb7266))
* **governor:** mem.status breakdown explains the cause of pressure; kernel pool and kernel load alerts (one notice per episode) ([4c69ca1](https://github.com/DevFranciscoCruz/axelstra/commit/4c69ca1be084b31a6df527e94e3e8b1c97955a58))
* **lock:** memory gate on available memory with a 2 GB margin (pure buildenv::mem_gate, setting build.lock_margin_mb) ([f850adf](https://github.com/DevFranciscoCruz/axelstra/commit/f850adf50bbf62db919b054bfc285ad26c075e27))
* **mem:** breakdown in axelstra mem and the memory panel (categories, kernel pool suggestion, CPU/commit, top CPU/IO, handles), kernel_pool_mb setting, docs ([6f4f843](https://github.com/DevFranciscoCruz/axelstra/commit/6f4f84389a3a0742d1c24f5b000c44924b073451))
* **prompts:** build-cost rules for leads, workers and the axelstration guide ([e552bac](https://github.com/DevFranciscoCruz/axelstra/commit/e552bac2f7d3280ae7f926134bbcfd042646586e))
* **scripts:** check.mjs --fast, jobs derived from cores and free memory, tools/measure-build.mjs ([97b12cc](https://github.com/DevFranciscoCruz/axelstra/commit/97b12cc816f9af7db0e477f45361d9f4b2b84648))


### Bug Fixes

* **app:** a running terminal keeps a stale daemon (offer restart instead of replacing) ([59cded8](https://github.com/DevFranciscoCruz/axelstra/commit/59cded80c4edbef465dec6b813bf8b8102427b5a))
* **app:** arm the auto pick before the worktree changes so selecting a worktree seats a tab ([4a04e90](https://github.com/DevFranciscoCruz/axelstra/commit/4a04e90505de0cb685a762452591f57c8e0ceab2))
* **app:** escape newline in localfs.io_failed, pt-BR wording, font-size token, prettier, single Channel import ([68079ae](https://github.com/DevFranciscoCruz/axelstra/commit/68079ae89f2557e71a3349698b3499622eb57fe8))
* **app:** import firstAgentLeads in App ([fe6acc7](https://github.com/DevFranciscoCruz/axelstra/commit/fe6acc7f44189a6bc295eb2e45bc8f04c00700db))
* **app:** import fmtFixed in MemoryPanel ([9fabf02](https://github.com/DevFranciscoCruz/axelstra/commit/9fabf02349453e1c57a5c664924b1ba035263674))
* **app:** only the user changes the active session; activate(id, by) is the single door, auto picks only seat an empty slot on restore/worktree select ([b16a5a7](https://github.com/DevFranciscoCruz/axelstra/commit/b16a5a79662d67b4dc846a2714e8b2d49f4757d1))
* **app:** project logo failure is local (6 s client timeout, silent letter fallback); redesigned error boundary whose retry remounts and refetches ([6aa310c](https://github.com/DevFranciscoCruz/axelstra/commit/6aa310c0af4e588c4c487d3b151c191b16bb3f55))
* **app:** re-issue lane reads that answer {loading: true} so callers never see it ([8b9e8c8](https://github.com/DevFranciscoCruz/axelstra/commit/8b9e8c83ef910fc4b6b4a3a07ebf9dfc97b83255))
* **app:** selecting a worktree row shows the worktree and no longer opens its agent ([6b57e01](https://github.com/DevFranciscoCruz/axelstra/commit/6b57e01fc6bc28360727812780064556be3bc9e2))
* **app:** selecting a worktree seats its main, never a task ([d806a91](https://github.com/DevFranciscoCruz/axelstra/commit/d806a91c3c2839117ed5fcf8c9aa6627da50fb0f))
* **app:** settings section list always visible, narrows instead of collapsing ([a69e105](https://github.com/DevFranciscoCruz/axelstra/commit/a69e1051e9e909a737293a2f743ae9f788a27f9d))
* **app:** sidebar main row and task rows select different things ([1be7e28](https://github.com/DevFranciscoCruz/axelstra/commit/1be7e2824c3c60791da1f38aa84a70852488f3de))
* **app:** start the first agent of a worktree the user creates as its main ([1f61949](https://github.com/DevFranciscoCruz/axelstra/commit/1f61949081eef160f79f5e7b0b5a441e310cd4a9))
* **build:** double zero in the memory-derived -Jobs (Math.Max overload overflowed with &gt;2 GB free) ([ac50e01](https://github.com/DevFranciscoCruz/axelstra/commit/ac50e01914ba3d4d74b3165f4726e0355ecd25fe))
* **check:** real-dir guard ignores settings.json ui keys rewritten by the running app ([1d7a25b](https://github.com/DevFranciscoCruz/axelstra/commit/1d7a25b6f0aab8c4ea710e3420edfb3ae56e2b25))
* **cli:** refuse dispatch send to a main checkout whose path has capitals on linux ([e21e775](https://github.com/DevFranciscoCruz/axelstra/commit/e21e7751b590a9a1345ea2b67cf569c1dcfbdfba))
* **core:** a repaint after resize or a terminal report is not activity ([2b5764f](https://github.com/DevFranciscoCruz/axelstra/commit/2b5764f14a113657dc756edf49cddd3c1315783f))
* **core:** count each child process once in the daemon's spawn statistics ([f91e94c](https://github.com/DevFranciscoCruz/axelstra/commit/f91e94c151693e63f1ffa46aede1521e19cb38be))
* **core:** name the Stamped table type (clippy type_complexity) ([5d22e8c](https://github.com/DevFranciscoCruz/axelstra/commit/5d22e8c21ca7ec6a8ebfa18baaaa9d5a350a2768))
* **daemon,app:** daemon calls end by event, not elapsed time ([6c72508](https://github.com/DevFranciscoCruz/axelstra/commit/6c72508e2f3cbf13317241c50661693a57315004))
* **daemon:** announce each bus message to an agent at most once ([19ed8a7](https://github.com/DevFranciscoCruz/axelstra/commit/19ed8a7805ee95b82975580589a016e005584c36))
* **daemon:** build governor before the Registry literal (E0382) ([0903837](https://github.com/DevFranciscoCruz/axelstra/commit/09038371be08a85d022a3c95b58c0c04700f0bd5))
* **daemon:** Defender hint helpers are Windows-only outside tests (unix clippy) ([73ec47a](https://github.com/DevFranciscoCruz/axelstra/commit/73ec47a58922ea71769a72f9e312aa0c5c41b8f3))
* **daemon:** find the worktree of an artifact sweep by any spelling of its path ([1934056](https://github.com/DevFranciscoCruz/axelstra/commit/1934056518620584c16ce35d299e890dfcd0b46b))
* **daemon:** mark session processes per registry so one teardown never kills another's session ([ca72385](https://github.com/DevFranciscoCruz/axelstra/commit/ca723854c4a08fbfddb58398873847d758e7dda8))
* **daemon:** memo never serves a pre-change answer at High after a projects/worktrees change ([4d47009](https://github.com/DevFranciscoCruz/axelstra/commit/4d47009a01ce1101d0c2db0298ef7a0d20ab6854))
* **daemon:** project.logo runs in its own lane with a 5 s answer limit and a hit/miss cache ([3616c18](https://github.com/DevFranciscoCruz/axelstra/commit/3616c18fbf543203707d45a59d91ee62894058d9))
* **daemon:** single-flight key per registry and project generation; plist_last only from a current list; watch test ignores usage.changed ([48b4bf0](https://github.com/DevFranciscoCruz/axelstra/commit/48b4bf00f1c118136747553206c9a89c04b2d1ec))
* **daemon:** stall watchdog sees real work (process tree, slot wait, messages), alerts the user only when nobody handles it; child notices no longer wipe the parent's state ([4dc71f1](https://github.com/DevFranciscoCruz/axelstra/commit/4dc71f116b15c20ac41af2a96a1c692e71b271a5))
* **daemon:** type a repeated done or escalation and a failed wake only once, without clearing the parent's state ([da09ced](https://github.com/DevFranciscoCruz/axelstra/commit/da09ced060a9b6bc39d6ceecfe3211944dfc5d13))
* **daemon:** usage watcher matches events by the root's canonical path too (FSEvents reports /private/var for /var) ([36852d6](https://github.com/DevFranciscoCruz/axelstra/commit/36852d6c1dc8e533820f29bf9dd0ff8b881455f7))
* **daemon:** wake on the first remote ref of a repository on linux ([9338f05](https://github.com/DevFranciscoCruz/axelstra/commit/9338f058ce015df69a85dcd3031c7456d119c6f5))
* **e2e:** dialogs are role=alertdialog now, find them by either role ([357025d](https://github.com/DevFranciscoCruz/axelstra/commit/357025dc92c893f8c5b8190b39db05a035ef16cd))
* **e2e:** type the remove-project word by field type, the dialog input no longer has a placeholder ([be3c699](https://github.com/DevFranciscoCruz/axelstra/commit/be3c69940eea466cc5eaf52e9c153e996dbecd72))
* **governor:** drop antivirus exclusion advice from the kernel pool notice ([08fd744](https://github.com/DevFranciscoCruz/axelstra/commit/08fd74409c189a4c4cabb2329b4d208211c0bddb))


### Performance

* app talks to the daemon over a lane pool; per-method timeouts name the slow method, ping tells slow from silent ([0b9ea32](https://github.com/DevFranciscoCruz/axelstra/commit/0b9ea32f6c56bfcd9d7da2637225bcbfc0ae9b73))
* **app:** coalesce project.list, overview, worktree.status, overlap and git.changes requests ([38dc46f](https://github.com/DevFranciscoCruz/axelstra/commit/38dc46fcd0394b6bc1c0e98140d66661138ff2fe))
* **app:** stop idle daemon traffic: createLive debounce/soft events/hidden skip, one shared usage loader, gated overview/ports/projects reads ([7b8a17d](https://github.com/DevFranciscoCruz/axelstra/commit/7b8a17d16edfb9f08ddd642264df63d9f00e1975))
* **build:** dev/test profile with line-tables-only debuginfo (none for deps); measured table in TOOLS.md ([4ec0320](https://github.com/DevFranciscoCruz/axelstra/commit/4ec032028c365ee7195dbf911b634667c7e19df8))
* **cli:** statusline spawns at most one process per tick ([f904051](https://github.com/DevFranciscoCruz/axelstra/commit/f90405137ad5d24c6a55c8e9a823839b4dd349d5))
* **daemon:** event-invalidated memo + single flight + bounded git pool for the idle slow reads ([aed2bcc](https://github.com/DevFranciscoCruz/axelstra/commit/aed2bcc4ab3e22eed897d057c74d411b41b33a5d))
* **daemon:** ignore ref events that leave the remote-tracking refs unchanged ([37bad44](https://github.com/DevFranciscoCruz/axelstra/commit/37bad44916f258b0218b44c17189e5613e4d8334))
* **daemon:** keep a worktree's git status and changes until its own file watch fires ([e10ca8a](https://github.com/DevFranciscoCruz/axelstra/commit/e10ca8af58978bf8b4ca5117b6535d057864a513))
* **daemon:** overview never runs gh; session.hint no longer drops the kept answers ([b53202e](https://github.com/DevFranciscoCruz/axelstra/commit/b53202e2431a5942e617bf74553c729958017795))
* **daemon:** re-read git only in the worktree that changed and announce projects.changed only on real changes ([5b8e21e](https://github.com/DevFranciscoCruz/axelstra/commit/5b8e21eab137a126bab503fd4af6fedb513427cb))
* **daemon:** resolve usage roots once, cache Gemini totals, single stat per usage file ([d06ca38](https://github.com/DevFranciscoCruz/axelstra/commit/d06ca38bd595b9488f36dc4f64a196d09a8e1c4f))
* **daemon:** reuse git answers while the repo's ref/index fingerprint holds (no spawn storm on memo bumps) ([119e385](https://github.com/DevFranciscoCruz/axelstra/commit/119e38514c16f32594064ce36b1c0652b66109c8))
* **daemon:** run slow reads in lanes, share identical in-flight calls, parallel project.list ([44a35d5](https://github.com/DevFranciscoCruz/axelstra/commit/44a35d5099b7f4e3ded53ffe995eee63fedfdf06))
* **daemon:** stop the janitor from re-waking itself and skip sweeps of unchanged repos ([4c959ed](https://github.com/DevFranciscoCruz/axelstra/commit/4c959ed9f36b0352039f0c19baf4a94687bc2296))


### Build System

* build-installer stamps the next version above the installed one ([b4bd670](https://github.com/DevFranciscoCruz/axelstra/commit/b4bd670da40f2bda6b82a85a6b71f1a56db34162))
* release-please config, manifest seeded at 0.1.42, next-version script ([d651b6d](https://github.com/DevFranciscoCruz/axelstra/commit/d651b6d07955983bb5bc3192bd9b84ffd55373c4))
* **windows:** VERSIONINFO and manifest for axelstra.exe and axelstra-app.exe ([940c370](https://github.com/DevFranciscoCruz/axelstra/commit/940c370e77764e21d116139eac2085c6f00b6cec))

## Changelog

All notable changes to Axelstra, newest first. Each section is written by release-please from the Conventional
Commit subjects merged since the previous release (see `docs/RELEASING.md`); the same text is the body of the GitHub
Release and of the update card in the app. Edit the release PR to adjust a section before it ships.
