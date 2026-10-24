# Security policy

## Reporting a vulnerability

Do not open a public issue. Report it privately through GitHub: the repository's **Security** tab,
**Report a vulnerability** (private security advisory). Include the version (`axelstra --version`),
your OS, and steps to reproduce.

Expect an acknowledgement as soon as the maintainer sees it. Fixes ship in the next release.

## Scope

- The desktop app, the daemon and the `axelstra` CLI.
- The daemon listens only on a local socket and authenticates clients with a shared secret stored in
  the per-user data directory (`token`). Anything that lets another local user or a remote party
  bypass that is in scope.
- The in-app updater only installs archives signed with the project's updater key.

Supported versions: the latest release only.
