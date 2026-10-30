# Windows code signing

How the Windows binaries can be Authenticode-signed, which provider fits this project, and what the owner has to do
by hand. The workflow side is in `.github/workflows/release.yml`; the release-day view (secret names, what happens
without them) is in `docs/RELEASING.md`. Antivirus false positives and how to report them are in `docs/ANTIVIRUS.md`.

Facts below were checked on 2026-10-29 against the linked pages. Prices, eligibility rules and product names change;
re-check the source before acting on them.

## What gets signed

The Windows release ships three executables, and all three are signed when a provider is configured:

| File | What it is | When it is signed |
| --- | --- | --- |
| `axelstra.exe` | the CLI and daemon (`crates/cli`), shipped as a Tauri sidecar | after `tauri build --no-bundle`, before the installer packs it |
| `axelstra-app.exe` | the Tauri GUI app | same step as the sidecar |
| `Axelstra_<version>_x64-setup.exe` | the NSIS installer | after `tauri bundle`; the updater `.sig` is then regenerated |

The workflow does not use Tauri's `bundle.windows.signCommand`. That hook runs one command per file inside
`tauri build`, which suits a synchronous signer (`signtool`, `trusted-signing-cli`), but the recommended provider
(SignPath) signs through an asynchronous request that a human approves. So the build is split instead:
`tauri build --no-bundle`, sign the two programs, `tauri bundle --bundles nsis`, sign the installer, re-sign the
updater signature, then check all signatures (including the two programs read back out of the installer with 7-Zip).

## Why sign at all

Signing does not make a program trusted by itself, and it is no fix for a real detection. It does three honest things:

- It says who published the file and proves the bytes were not changed after the release build.
- Windows SmartScreen and Microsoft Defender attach reputation to the signer, not only to the file hash. An unsigned
  file starts with no identity at all, so every new build is a stranger.
- Machine-learning antivirus engines (Defender's cloud ML verdicts such as `Trojan:Win32/Bearfoos.A!ml`, or engines
  like Trapmine on VirusTotal) weigh "unsigned" as one of many features. An unsigned, new, rarely seen executable
  that starts processes, reads other processes' memory statistics or suspends processes (all of which the memory
  governor legitimately does) is a typical ML false positive. A valid signature from a known publisher lowers that
  score. It does not guarantee anything: signed software still gets ML false positives, and then the fix is a
  false-positive submission, described in `docs/ANTIVIRUS.md`.

Signing is transparency only. It must never be paired with packing, obfuscation or anything that hides what the
program does.

## The options

### 1. SignPath Foundation (recommended)

Free code signing for open-source projects: SignPath.io provides the signing service, the certificate is issued to
SignPath Foundation. Sources: <https://signpath.org/>, <https://signpath.org/terms>,
<https://docs.signpath.io/trusted-build-systems/github>, <https://docs.signpath.io/origin-verification>.

- **Cost:** free for OSS projects ("For OSS projects, our services are free of charge", signpath.org).
- **Who appears as publisher:** *SignPath Foundation*, not the owner. "The code signing certificate is issued to
  SignPath Foundation. This means that SignPath Foundation is the publisher of the OSS project" (terms). Microsoft
  describes it as "OV-level certificate signing through a managed pipeline"
  (<https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options>).
- **Conditions** (terms page; the code of conduct is still marked draft): OSI-approved license without commercial
  dual licensing (MIT fits), no proprietary components, actively maintained, already released in the form to be
  signed, functionality documented on the download page; sign only your own project, built from source "in a
  verifiable way"; no hacking tools; software that sends user data elsewhere needs a privacy policy shown at install
  and an opt-out; no system changes without warning; an uninstaller; MFA for every team member on SignPath and GitHub;
  team roles (authors, reviewers for outside pull requests, approvers); "Every release needs manual approval for
  signing"; a **code signing policy** page on the project website (home and download pages) with the text "Free code
  signing provided by SignPath.io, certificate by SignPath Foundation", the team roles and a privacy statement; the
  artifact configuration must enforce the product name and an identical product version on every signed file
  (file metadata restrictions, which is why the Windows version resources matter).
- **Fit for Axelstra:** MIT, public GitHub repository, releases built by GitHub Actions on GitHub-hosted runners
  (SignPath's origin verification ties each signing request to the repository, branch, commit and workflow run).
  Points to settle before applying: the privacy statement (list every network call the app and daemon make on
  their own, such as the updater check, and what each sends) and the warnings the installer gives. The memory governor's process
  suspension is a documented, user-visible feature and stays as it is.
- **What the owner does by hand:**
  1. Write the code signing policy page and the privacy statement (website or a page in the repository that the
     README links as "Code signing policy").
  2. Apply at <https://signpath.org/apply> (the form was not readable when this was written; follow the page).
  3. After acceptance, in SignPath: create or confirm the project (slug `axelstra` by default), connect the
     GitHub repository as a trusted build system, and create two artifact configurations, slugs `binaries` and
     `installer` (sketch below; add the product name/version metadata restrictions SignPath requires).
  4. Create a CI user / API token with submitter rights on the release signing policy. Enable MFA everywhere.
  5. Add `SIGNPATH_API_TOKEN` and `SIGNPATH_ORGANIZATION_ID` as secrets of the `release` Environment. Optional
     variables: `SIGNPATH_PROJECT_SLUG` (default `axelstra`), `SIGNPATH_POLICY_SLUG` (default `release-signing`).
  6. On each release, approve the two signing requests in SignPath (binaries, then installer). The job waits up to
     30 minutes for each.
- **Secrets in the workflow:** `SIGNPATH_API_TOKEN`, `SIGNPATH_ORGANIZATION_ID`.
- **Effect:** valid Authenticode signature on all three files with a publisher that already has reputation from many
  projects. SmartScreen reputation still builds per file and publisher over time (no instant trust for anyone since
  2024, see below). ML engines see a signed file from a known publisher instead of an unsigned one.
- **Drawbacks:** the publisher shown in Windows dialogs is SignPath Foundation; every release needs a manual
  approval in SignPath; the conditions above are SignPath's to change, and SignPath may revoke on violations.

Artifact configuration sketch (check the element names and the metadata restriction syntax against SignPath's
artifact configuration docs before saving; not tried against a live SignPath project):

```xml
<!-- slug: binaries (the zip GitHub makes from sign-in/binaries) -->
<artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
  <zip-file>
    <pe-file path="axelstra-app.exe"><authenticode-sign/></pe-file>
    <pe-file path="axelstra-x86_64-pc-windows-msvc.exe"><authenticode-sign/></pe-file>
  </zip-file>
</artifact-configuration>

<!-- slug: installer -->
<artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
  <zip-file>
    <pe-file path="*-setup.exe"><authenticode-sign/></pe-file>
  </zip-file>
</artifact-configuration>
```

### 2. Azure Artifact Signing (formerly Azure Trusted Signing)

Microsoft's managed signing service. It was renamed from Trusted Signing to **Artifact Signing** in early 2026 with
"no functional difference" (<https://azure.microsoft.com/products/artifact-signing>). The GitHub action moved from
`azure/trusted-signing-action` to `azure/artifact-signing-action` (the old name redirects; the workflow pins
`v2.0.0`). Sources: <https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart>,
<https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options>,
<https://azure.microsoft.com/en-gb/pricing/details/trusted-signing/>, <https://github.com/Azure/artifact-signing-action>.

- **Eligibility:** public-trust certificates for **organizations** in the USA, Canada, the EU, the UK and a few other
  countries; **individual developers only in the USA and Canada** (quickstart, updated 2026-10-08). A Microsoft Q&A
  answer says an unincorporated sole trader in the EU does not qualify either
  (<https://learn.microsoft.com/en-us/answers/questions/5915697/can-an-irish-sole-trader-(non-incorporated)-qualif>).
  **A solo owner in Portugal is not eligible as an individual today.** Only a registered organization (a company
  with its legal details, a website and a domain e-mail) would be.
- **Price:** Basic about USD 9.99/month for 5,000 signatures, USD 0.005 per extra signature (pricing page).
- **Identity validation:** organization validation in the Azure portal, 1 to 20 business days; documents issued in the
  last 12 months. Certificates are short-lived and renewed by Microsoft; no hardware token to manage.
- **What the owner does by hand:** Azure subscription; Artifact Signing account; identity validation; certificate
  profile; an app registration (service principal) with the signer role on the certificate profile (the role is
  called *Artifact Signing Certificate Profile Signer*, formerly *Trusted Signing Certificate Profile Signer*; check
  the name in the portal); a client secret.
- **Secrets in the workflow:** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `TS_ENDPOINT` (the account's
  regional endpoint, for example `https://weu.codesigning.azure.net`), `TS_ACCOUNT`, `TS_PROFILE`. The workflow
  supports this provider as the second choice (used when the SignPath secrets are absent). It was not run yet.
- **Effect:** publisher is the validated organization. Microsoft says it "does not provide instant SmartScreen trust";
  reputation builds with consistent signing. Same effect on ML engines as any reputable signature.

### 3. Classic OV or EV certificate from a CA

- **Cost:** OV about USD 150–300/year, EV USD 400+/year (Microsoft's comparison page above; CA list prices vary).
- **Key storage:** since 1 June 2023 the CA/Browser Forum Code Signing Baseline Requirements require the private key
  to be generated and used in a hardware crypto module (FIPS 140-2 Level 2 or Common Criteria EAL 4+), for OV and EV
  (<https://cabforum.org/working-groups/code-signing/requirements/>). A key in a `.pfx` file as a GitHub secret is no
  longer possible. For CI that means a **cloud HSM / remote signing** service: the CA's own (for example DigiCert
  KeyLocker, SSL.com eSigner, GlobalSign's cloud offering) or a key in Azure Key Vault Premium / Managed HSM or Google
  Cloud KMS HSM with a CA that accepts the attestation. These usually cost extra per year or per signature. A USB
  token works only on a self-hosted runner with the token plugged in.
- **Validity:** certificates issued since 1 March 2026 last at most 460 days (ballot CSC-31,
  <https://cabforum.org/working-groups/code-signing/requirements/>), so renewal and revalidation are yearly.
- **Individuals:** many CAs sell OV to individuals (validated with a government ID); EV is for organizations
  (some CAs offer a sole-proprietor variant). Available worldwide, so this works from Portugal.
- **SmartScreen today:** EV no longer gives instant reputation. Microsoft removed the EV special treatment in 2024;
  "EV now has the same reputation process as OV" (Microsoft's comparison page;
  see also <https://learn.microsoft.com/en-us/answers/questions/1846647/program-requirements-microsoft-trusted-root-program>).
  The date given varies between sources (March or August 2024). There is no reason to pay for EV for SmartScreen.
- **What the owner does by hand:** buy the certificate, pass validation, set up the cloud HSM account, renew every year.
- **Secrets:** depend on the vendor (API user, password, TOTP seed or credential ID for eSigner; client certificate
  and API key for KeyLocker). Not implemented in the workflow; it would be a third provider step with the vendor's
  signing tool, using the same split build.
- **Effect:** publisher is the owner's name; reputation starts at zero and builds over time, like the other options.

## Recommendation

**SignPath Foundation.** It is free, made for exactly this case (MIT, public GitHub repository, builds on GitHub
Actions), needs no company, and is available to an individual in the EU, which Azure Artifact Signing is not. The
price is that the publisher shown is SignPath Foundation and that each release needs a manual approval in
SignPath, which fits a release flow that already has a manual Environment approval.

If the owner later registers a company, Azure Artifact Signing becomes the cheapest way to sign in the owner's own
name, and the workflow already supports it. A classic OV certificate with a cloud HSM is the fallback if neither is
possible; it is the most expensive and the most work.

## What signing does not do

- It does not clear an existing ML detection. Submit the false positive (see `docs/ANTIVIRUS.md`) for the exact
  file; a signed file is easier for the vendor to whitelist by certificate.
- It does not give instant SmartScreen trust with any certificate type.
- It does not cover local builds. Local release builds of the same code stay unsigned and can still be flagged.
