# Antivirus false positives

On 2026-10-29 two machine-learning (reputation) engines flagged Axelstra binaries. Neither verdict comes from a
signature that matches code in Axelstra; both are classifier scores. This file records what was detected, what was
tested to find the cause, and how to report the false positive. It changes nothing in what the program does: the fix is
transparency (version resources, manifest, symbols, signing, this documentation), never packing, obfuscation, string
encryption, anti-analysis or hidden API calls.

## Conclusion (2026-10-29)

**Nothing tested changed the verdict, because the original verdict could not be reproduced.** With real-time protection,
on-access protection and behavior monitoring on again (from 21:10, definitions 1.463.22.0, engine 1.1.26100.4, cloud
protection on), no build was flagged on write, on start (`axelstra.exe --version`) or by an on-demand scan:

- commits `ca1d444` (installed 0.1.42), `1a54aab`, `0903837` (governor part 1, first buildable), `e15533c` (governor
  part 2: `NtSuspendProcess`/`NtResumeProcess`), `c9c7bbd` (`core::proc::hidden`), `4c69ca1` (cause breakdown),
  `c7fd4f1` (CLI only) / `9fabf02` (pool tags via `NtQuerySystemInformation`), `d06ca38` (the reported build), `940c370`
  (VERSIONINFO + manifest on both exes);
- release profile variants on `940c370`: current (`opt-level "s"`, `lto true`, `codegen-units 1`, `strip true`),
  `strip = false`, `strip = "debuginfo"`, `opt-level 3` + `lto "thin"`, and the CI shape (`strip "none"`,
  `debug "line-tables-only"`, `split-debuginfo "packed"`);
- the installed 0.1.42 `axelstra-app.exe`, the published `Axelstra_0.1.42_x64-setup.exe`, and the repository's own
  `target\release\axelstra.exe` of `d06ca38` built at 19:07.

The installed 0.1.42 `axelstra.exe` was unchanged from 2026-10-26 and was only flagged at 18:26 on 2026-10-29, so the
verdict changed on Microsoft's side (a cloud ML / `FastPath` decision about specific files), not because of a code
change. Every `axelstra.exe` hash is new per build and none of the new hashes got that verdict. No commit and no profile
setting can be named as the cause. The profile in `Cargo.toml` is left as it was. The VERSIONINFO and manifest
(`940c370`) are good transparency and stay, but they cannot be shown to have fixed anything. The right next step is the
Microsoft false-positive submission below, with the hash of the file that was actually quarantined (the 0.1.42 release
asset), plus Authenticode signing so reputation builds on the publisher rather than per hash.

Trapmine (`axelstra-app.exe`, VirusTotal only) cannot be tested locally; its hashes are in the table for re-upload.

## What was detected

| Engine | Verdict | File | Notes |
| --- | --- | --- | --- |
| Microsoft Defender (cloud ML) | `Trojan:Win32/Bearfoos.A!ml` (threat id 2147731250), definitions 1.463.22.0, engine 1.1.26100.4 | `axelstra.exe` (CLI and daemon, `crates/cli`) | Installed 0.1.42 ran without trouble from 2026-10-26 until 18:26 on 2026-10-29, then was quarantined. A local release build of `d06ca38` was quarantined too. |
| Trapmine (on VirusTotal only) | `Malicious.moderate.ml.score` | `axelstra-app.exe` (Tauri app, installed 0.1.42, commit `ca1d444`) | Seen only on VirusTotal; Trapmine is not installed locally. |

Defender's own record of the detections (`Get-MpThreatDetection`, event log
`Microsoft-Windows-Windows Defender/Operational`, ids 1116/1117):

| Time | File | Process that touched it | Detection type | Result |
| --- | --- | --- | --- | --- |
| 18:26:11, 18:26:21 | `D:\Programas\Axelstra\axelstra.exe` (installed) | `axelstra-app.exe` (the app starting its daemon) | real-time protection | quarantined |
| 18:30:22 | `<repo>\target\release\axelstra.exe` | `cargo.exe` (writing the build output) | real-time protection, `FastPath` | quarantined |
| 19:11:12 | `D:\Programas\Axelstra\axelstra.exe` and two running `axelstra.exe` processes | `axelstra.exe` | real-time protection | action failed (file in use) |
| 19:11:22 | `D:\Programas\Axelstra\axelstra.exe` | `bash.exe` | real-time protection, `Concrete` | quarantined |

Every detection has `DetectionSourceTypeID` 3 (real-time protection): Defender judged the file when a process wrote or
started it. `FastPath` is a dynamic signature delivered by the cloud protection service for a specific file, which is how
a `!ml` cloud verdict reaches the local engine. `axelstra-app.exe` itself was never flagged by Defender; it appears only
as the process that started the flagged `axelstra.exe`.

## How the scans were done, and their limits

Builds were made in throwaway worktrees (`git worktree add --detach`) under `%TEMP%\axbisect\<sha>`, each with its own
`CARGO_TARGET_DIR`, through `axelstra lock verify`, in the same way as `scripts/build-installer.ps1` without the NSIS step:
`cargo build --release -p axelstra`, copy it to `app/src-tauri/binaries/axelstra-x86_64-pc-windows-msvc.exe` (the
`externalBin` sidecar of `tauri.release.conf.json`), `npm ci`, `npm run build`, then
`npx tauri build --no-bundle --config src-tauri/tauri.release.conf.json`. Both exes were copied to
`%TEMP%\axscan\<label>\`, hashed with `Get-FileHash` (SHA-256) and scanned with
`Start-MpScan -ScanType CustomScan -ScanPath <folder>` and
`MpCmdRun.exe -Scan -ScanType 3 -File <exe> -DisableRemediation`. `%TEMP%` is not excluded from Defender.

**Limit: the original detection could not be reproduced on this machine.** Real-time protection was switched off at
19:11:55 (event 5001) and stayed off until 21:10; the first round of scans (until about 21:00) ran in that window
(`Get-MpComputerStatus`: `RealTimeProtectionEnabled` False). Those were all repeated with real-time protection on (see
Results). What follows describes the first round. The exclusions for the install folder, the repository's `target` and `.axelstra-wt` were added at 19:19 (event
5007). So every result here is an on-demand scan, with cloud protection on (`MAPSReporting` 2,
`SubmitSamplesConsent` 1, `CloudBlockLevel` 0) but without the on-write / on-start check that produced all four real
detections. Starting `axelstra.exe --version` does nothing for Defender while real-time protection is off. Adding a
Mark-of-the-Web (`Zone.Identifier` stream, `ZoneId=3`) and rescanning did not change any result either. No positive
control was available: the quarantined installed `axelstra.exe` cannot be listed or restored without administrator
rights (`MpCmdRun -Restore -ListAll` returns 0x80070005), and the repository's `target\release\axelstra.exe` built at
19:07 (sha256 `1EBA657BBDC74EB5BAAF51C6AF843E5327A599BBF526977F5AEA6ECDE6BD0BDF`) scans clean on demand. A clean on-demand
scan therefore does **not** prove that a build would pass the real-time check; it only shows that the static engine and
the on-demand cloud lookup do not flag it.

**Method with real-time protection on (from 21:10).** For each build, the exe was copied fresh with `File.Copy` from
`%TEMP%\axscan\<label>\` into a new folder `%TEMP%\axscan\oa\<label>\` (on-write check), checked 8 s later, then
`axelstra.exe --version` was started from there (on-start check; `axelstra-app.exe` was never launched, so its on-start
column is n/a), checked again, scanned with `MpCmdRun -Scan -ScanType 3 -File <exe> -DisableRemediation`, and
`Get-MpThreatDetection` plus events 1116/1117 were read for that folder since the copy. The table repeats nothing from
the earlier runs with real-time protection off; those are superseded.

## Results

All rows: on-write = kept, on-start = ran, exit 0, on-demand = no threats, 0 detections, no 1116/1117 events.

| Label | Commit | What it added / variant | File | SHA-256 |
| --- | --- | --- | --- | --- |
| installed-0.1.42 | `ca1d444` | installed release 0.1.42 (CI build) | axelstra-app.exe | `B42A76AF071147CEB6389E7BC30173BF0B37E6E734B0BD53A53878CB4EB1C35A` |
| rel-0.1.42 | `ca1d444` | published `Axelstra_0.1.42_x64-setup.exe` (on-write + on-demand only) | setup.exe | `1BC5A5227D8447785C962D40D6BB56D3730B1E5E32A42FB79D66F02BD97872F1` |
| ca1d444 | `ca1d444` | baseline (0.1.42 source), local build | axelstra.exe | `B414CC050220E5F7247C6686265542EEDDFAA9C6C7F42FBF7098D1DA1B668594` |
| ca1d444 | `ca1d444` | | axelstra-app.exe | `E4ABF3E23A8E31AB5552901393AB882119D23AA831483A352F15D82F480C6D95` |
| 1a54aab | `1a54aab` | scanx wip (later removed its Defender part, `3288f3c`) | axelstra.exe | `6974323C978DA8BC8E9F84568359F1AE5C722238117C83E05C12BD9F75AC171E` |
| 1a54aab | `1a54aab` | | axelstra-app.exe | `38A8E832D63A621D68EEDA6F7D61C7792809456304E997BC5F6E71272FC27134` |
| 0903837 | `0903837` | governor part 1 (`0b2c94b` does not compile, E0382; this is its fix) | axelstra.exe | `480BF98088814AE29CF921B2FDDDE9568B83346BE3C5F17858CF0B70236A4BC1` |
| 0903837 | `0903837` | | axelstra-app.exe | `10343065324813A07773ED67AEF4C5D2AACB9F66801908AE5F0923249A5B7436` |
| e15533c | `e15533c` | governor part 2: `NtSuspendProcess`/`NtResumeProcess` | axelstra.exe | `D1DE5F3CAB801FF73F80BFBFED4A9A8A54701279522CA2BC4C205A1945A6840A` |
| e15533c | `e15533c` | | axelstra-app.exe | `DE7BE4DC01C562C0BE06A2C5E3DDDCAEB8C90D7D5008A4EF5BD24AD33A0E37D2` |
| c9c7bbd | `c9c7bbd` | hidden child processes `core::proc::hidden` | axelstra.exe | `F8293456601D67FE877F0AD5C5EA6443A70C5A8C888D30D64A3F34A5C59310CE` |
| c9c7bbd | `c9c7bbd` | | axelstra-app.exe | `6EAF62451D8DF2D69DA19DB8FCBD8EF51D5C02E565F789B77FA3DF1FC8CAF19D` |
| 4c69ca1 | `4c69ca1` | memory cause breakdown | axelstra.exe | `468FF2FEB4D571E532F9FD680F12B726E57BD789C63B45A95B7BADC7DE68E392` |
| 4c69ca1 | `4c69ca1` | | axelstra-app.exe | `58B1CF063536CC565D6D25715099FBD1EC5D951B8B06AEE13F60BC8AB2E21681` |
| c7fd4f1 | `c7fd4f1` | pool tags via `NtQuerySystemInformation` (app does not compile: `fmtFixed`) | axelstra.exe | `9095F9BBD8CCC4C7C3DF7686CB53F6232A384F43D4C59E5287E480881EA8B984` |
| 9fabf02 | `9fabf02` | pool tags, app fixed | axelstra.exe | `612FA7D6E96C82E8DB1FD4A8C3567B400E9F036DC16324CE84E6ED7813D3B6AA` |
| 9fabf02 | `9fabf02` | | axelstra-app.exe | `7AB91D3A2D0B779775EFC8E00A5E5BD9AF78B769EC9A4EF7059B4C1AD2332D22` |
| d06ca38 | `d06ca38` | the reported build | axelstra.exe | `AED3D012889A220A43A35E15ABEF2D907FB7D4D0DAFF05AE07B2F7245A32155C` |
| d06ca38 | `d06ca38` | | axelstra-app.exe | `526208D8F566296E2A7D6EC21DD7F8BC473D9D1AD03BE8F7E1EE5DC6C3E49689` |
| repo-target-1907 | `d06ca38` | the repository's own `target\release` build of 19:07 | axelstra.exe | `1EBA657BBDC74EB5BAAF51C6AF843E5327A599BBF526977F5AEA6ECDE6BD0BDF` |
| head-940c370 | `940c370` | VERSIONINFO + manifest, current profile | axelstra.exe | `6553709F6AF14DC294A08B34D580F27DE45CEEFF0C0D16E6BA929917AA195911` |
| head-940c370 | `940c370` | | axelstra-app.exe | `30DE46FE5CC7D2FD3DADE71EA551CEAE818098311406CD95206A52D837868CC7` |
| v-nostrip | `940c370` | `strip = false` | axelstra.exe | `1FE2576214F557B8AF3E8400DBE94432413719AC01CB02EB733F18C7A8F93801` |
| v-nostrip | `940c370` | | axelstra-app.exe | `375C6BF222ABFB11DA47416C33C8E9382760BA5FCCEEA6E791CE7D2C04EF6A4C` |
| v-debuginfo | `940c370` | `strip = "debuginfo"` | axelstra.exe | `F97FF06CA58818E715332495BF15C6428E1807D8C90A1F853B874A9921FFDEDE` |
| v-debuginfo | `940c370` | | axelstra-app.exe | `728F819E0F0DEE11FF9C9895B3FF7598EE90A8AA9A7018747548AF272695EEA2` |
| v-o3thin | `940c370` | `opt-level = 3`, `lto = "thin"` | axelstra.exe | `4A3C6A32260BF3434AA88BCA182D93ECE88F53D52580B5138F6477D4A0B96229` |
| v-o3thin | `940c370` | | axelstra-app.exe | `7419D78BCBCE418ADA5A17F3C7F54F5B710F873B5F207AB2BD438DFF2E62D2ED` |
| v-cilike | `940c370` | CI shape: `strip "none"`, `debug "line-tables-only"`, `split-debuginfo "packed"` | axelstra.exe | `E87F0BA598DEB52EF2DF7EA44CA6CD0F99304DDB7B0B5DF16008D75A44DBF3A0` |
| v-cilike | `940c370` | | axelstra-app.exe | `D95AE0908844994D5F62873052A677845021438DD0E24B938AF5CBE6501D0050` |

Builds were made with `CARGO_BUILD_JOBS` 1 to 7 as the machine allowed; one variant build was frozen by the memory
governor for about 30 minutes at Critical and resumed by itself. The installed 0.1.42 `axelstra.exe` (the file actually
quarantined) could not be tested: it is in quarantine and restoring needs administrator rights.

## What is inside the binaries (task 3)

- `axelstra-app.exe` 0.1.42 (installed): VERSIONINFO from Tauri (FileVersion 0.1.42, Company `axelstra`, no copyright,
  no OriginalFilename); manifest with only the Common-Controls v6 dependency (no `requestedExecutionLevel`, no
  `supportedOS`); not signed; PDB name `axelstra_app.pdb`. The WebView2 loader is linked statically (no
  `WebView2Loader.dll` in the install folder; the `EdgeUpdate` lookup strings are inside the exe). ONNX Runtime / sherpa
  (speech) is linked statically, which is a large part of the file. It contains exactly one PE image (no embedded copy
  of `axelstra.exe`); it starts `axelstra.exe daemon` from its own folder (`start_daemon` in `app/src-tauri/src/main.rs`)
  with `DETACHED_PROCESS | CREATE_NO_WINDOW`. That launch is what the 18:26 detections record as the process. The
  bundler ships `axelstra.exe` as an `externalBin` sidecar (`tauri.release.conf.json`).
- `axelstra.exe` before `940c370`: no VERSIONINFO, no manifest, not signed.
- `940c370` (both exes): full VERSIONINFO (Company `Francisco Cruz`, description, OriginalFilename, MIT copyright;
  version 0.1.0 in an unstamped tree) and a manifest with `asInvoker`, Windows 10/11 `supportedOS` and `longPathAware`.
  Still not signed.

## What the binaries do (verified against the code)

Use this text when reporting the false positive. Every statement was checked in the source at the commit named in the
report.

> Axelstra is an open-source (MIT) desktop orchestrator for coding agents and terminals:
> https://github.com/DevFranciscoCruz/axelstra. `axelstra.exe` is its command-line tool and local daemon;
> `axelstra-app.exe` is the Tauri (WebView2) window that starts `axelstra.exe daemon` from its own folder and talks to it.
>
> - The daemon starts child processes without a console window (`CREATE_NO_WINDOW`): git, gh, the user's shells,
>   and coding-agent CLIs (Claude Code, Codex, Gemini) inside pseudo-terminals the user sees in the app. Their process
>   trees are placed in a Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so they end with the daemon.
> - Under memory pressure it can pause process trees of its own sessions with `NtSuspendProcess` and resume them with
>   `NtResumeProcess`. It never touches processes outside its sessions. Every paused pid is recorded and resumed on
>   shutdown, on any exit of the server loop, from the panic handler, and at the next start from a saved list.
> - It reads memory statistics read-only: `GetPerformanceInfo`, `GlobalMemoryStatusEx`, `GetProcessMemoryInfo`, and
>   `NtQuerySystemInformation` (kernel pool tag usage, class 22) to explain where memory went. Nothing is written.
> - It listens only on a local named pipe, `\\.\pipe\axelstra-<user SID>`, with a DACL that grants access to the
>   current user and SYSTEM only; clients also present a per-user token stored in the user's profile. It opens no
>   network port and sends no telemetry. Network traffic comes only from the tools it starts on the user's request
>   (git, gh, ssh, the agents), and in the app from the updater (GitHub releases `latest.json`) and an optional
>   speech-model download from Hugging Face.
> - Optional and audited: a screen-control feature for agents (window list, screenshots with `PrintWindow`/`BitBlt`,
>   mouse and keyboard with `SendInput`). It is off until the user switches it on in the app, and it is off again
>   every time the daemon starts.
> - The app writes one per-user registry value under `HKCU\Software\Microsoft\DirectX\UserGpuPreferences` (the
>   same switch as Settings > Display > Graphics) on machines with two GPUs, and never overwrites an existing choice.
>
> The binaries are not packed or obfuscated and contain no anti-analysis code.

## Reporting a false positive

### Microsoft Defender

1. Open https://www.microsoft.com/wdsi/filesubmission and sign in.
2. Choose **Software developer** (you build the software), product **Microsoft Defender Antivirus (Windows 10/11)**.
3. Upload the exact flagged file (the installed `axelstra.exe`, or the release asset), or a zip of it.
4. Choose **Incorrectly detected as malware/malicious**.
5. Detection name: `Trojan:Win32/Bearfoos.A!ml`. Definition version: the one in the alert (`1.463.22.0` for the
   2026-10-29 detections; `Get-MpComputerStatus | Select AntivirusSignatureVersion` shows the current one).
6. Paste the description from "What the binaries do" with the version, commit and SHA-256 of the file.

After Microsoft clears it, update the definitions (`Update-MpSignature`, or Windows Security > Virus & threat
protection > Protection updates) before testing again.

### Trapmine on VirusTotal

1. On the file's VirusTotal page, click **Reanalyze** (the circular arrow). ML verdicts can change between engine
   versions, and a fresh scan is what a vendor will look at first.
2. If Trapmine still flags it, contact the vendor with the VirusTotal link, the SHA-256, the detection name and the
   description above. VirusTotal only aggregates verdicts and cannot change them. The vendor contacts are listed at
   https://docs.virustotal.com/docs/false-positive-contacts (Trapmine has an entry there; its e-mail address is shown
   on that page only, and it is not repeated here because it could not be confirmed from this machine). Send the hash
   and the link rather than the executable: some vendors' mail servers reject attachments with executables.
