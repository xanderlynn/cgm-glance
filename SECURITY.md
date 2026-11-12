# Security

## How sign-in details are handled

- Dexcom usernames and passwords, and Nightscout tokens, are stored **only** in the operating
  system's keychain: Windows Credential Manager, macOS Keychain, or Linux Secret Service.
  The keychain entry is named `cgm-glance` with a random id per person.
- The settings file (`config.json` in the app's config folder) never contains sign-in details.
  A unit test fails the build if it ever could.
- If no keychain is available (some minimal Linux setups), the app refuses to save instead of
  falling back to a file.
- Secrets never go back to the app's windows. They are typed once, tested, and stored.
- On a wrong Dexcom password the app stops trying, so it can't lock the account.
- The app only connects to the Dexcom Share host for the chosen region or the user's own
  Nightscout site, over HTTPS. No analytics or tracking.
- Redirects are followed only on the same host, so a token or session id can't be sent
  to another server. Answers larger than 512 KB are refused.
- After Dexcom rejects a password in the setup wizard, the next attempt must wait 20 s.
- The settings file and its folder are readable only by the current user (on macOS and
  Linux). An unreadable settings file is kept as `config.json.unreadable`, not overwritten.
- Windows can only listen for app events, not send them, and load only the app's own pages
  (strict Content-Security-Policy).
- Each window can call only the commands it needs (`src-tauri/capabilities/`). Desktop
  badges can only read the current readings. Only Settings can test or save sign-ins. A
  test fails if a page calls a command its window isn't granted.
- Alarm notifications can leave out glucose numbers (Settings → Alarms), keeping them off
  the lock screen and out of notification history.

## Known limits

- The Nightscout access token travels in the request URL, as Nightscout's API expects. It
  stays within HTTPS to your own site, but your site's access logs may record it. Use a
  token with only the "readable" role.
- Installers aren't code-signed (Windows Authenticode, macOS notarization) yet, so the OS
  warns on first install. In-app updates are signed separately (see below).
- Pages allow inline `<style>` blocks (`style-src 'unsafe-inline'`). Scripts are limited to
  the app's own files, and the pages never insert HTML from data.
- Some Linux-only libraries pulled in by Tauri and the keyring are flagged as unmaintained
  by `cargo audit`. None have known vulnerabilities; updates come from upstream.

## This repository

- No real credentials, tokens or patient data belong here. Tests use synthetic data only.
- CI runs [gitleaks](https://github.com/gitleaks/gitleaks) and `cargo audit` on every push
  and pull request. GitHub secret scanning with push protection, Dependabot alerts and
  CodeQL are on. Dependabot proposes updates weekly.
- GitHub Actions must be pinned to exact commits (enforced in the repository settings).
  Workflows get read-only tokens unless a job needs more.
- `main` accepts changes only through pull requests that pass CI, and can't be force-pushed
  or deleted. Only maintainers can create or move release tags (`v*`).
- `.gitignore` excludes local config files and common key formats.
- In-app updates are fetched over HTTPS from this repository's GitHub Releases and installed
  only if their signature matches the public key built into the app. The private update key
  exists only as a secret of the `release` environment (and with the maintainer), never in
  the repository. That environment accepts only `v*` tags and needs a maintainer's
  approval before each build, and the build checks the tag against the app version. Draft
  releases aren't served, so nothing ships until a release is published.

## Reporting a problem

Please report privately through GitHub (Security → Report a vulnerability), not a public
issue.
