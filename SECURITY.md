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

## This repository

- No real credentials, tokens or patient data belong here. Tests use synthetic data only.
- CI runs [gitleaks](https://github.com/gitleaks/gitleaks) on every push and pull request.
- `.gitignore` excludes local config files and common key formats.

## Reporting a problem

Please open a private security advisory on GitHub (Security → Report a vulnerability)
rather than a public issue.
