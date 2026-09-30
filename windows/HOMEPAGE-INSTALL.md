# Homepage text: installing Inkwell on Windows (draft)

For the release-prep step, which owns the homepage: the text of its Windows install section while
the Windows build is not code-signed. `X.Y.Z` is the release's version (the homepage's
`APP_VERSION`); the file names are the ones `.github/workflows/win-release.yml` publishes. The
Windows-specific UI labels below were written from Microsoft's documentation and need checking on
a PC once (windows/S3.6-CHECKLIST.md, section 1) before this goes live.

---

## Windows

Inkwell for Windows 11 (version 24H2 or later), 64-bit. It installs for your user account alone:
no administrator password, nothing system-wide.

1. **Download** `Inkwell_X.Y.Z_x64-setup.exe` from the [latest release](https://github.com/SirSicard/inkwell/releases/latest).
   Your browser may say the file "isn't commonly downloaded". In Edge, open the download's **…**
   menu, choose **Keep**, then **Show more** and **Keep anyway**. In Chrome, choose **Keep**.

2. **Check the download** (recommended). The Windows build is not code-signed yet, so this is how
   you know the file is the one we published. In PowerShell:

   ```powershell
   Get-FileHash -Algorithm SHA256 "$HOME\Downloads\Inkwell_X.Y.Z_x64-setup.exe"
   ```

   The hash it prints must match the one in the release notes, and in
   `Inkwell_X.Y.Z_windows-sha256.txt` on the same release. If it does not, delete the file and do
   not run it.

3. **Run it.** Windows SmartScreen will say "Windows protected your PC", with the publisher
   "Unknown publisher". Choose **More info**, then **Run anyway**. Inkwell installs in a few seconds
   and opens; it is in the Start menu as **Inkwell**.

   If Windows says **Smart App Control** blocked it, there is no "Run anyway": Smart App Control
   does not run apps that are not signed. The signed build is on its way; we would rather you wait
   for it than turn off a protection you chose to keep.

**Updates.** Inkwell checks for a new version when you ask it to: Settings > About > **Check Now**.
Updates come from this project's GitHub releases over HTTPS, and Inkwell checks each one's size and
SHA-256 against the release before installing it.

**Uninstalling.** Settings > Apps > Installed apps > Inkwell > **Uninstall**. Your library (meetings,
dictations, notes and downloaded models) stays in `%LOCALAPPDATA%\Inkwell`, so a reinstall picks up
where you left off. To remove it as well, delete that folder after uninstalling.

**Microsoft's terms.** Inkwell includes the runtime of Microsoft's Windows App SDK, the Windows
SDK's .NET projection and the Visual C++ runtime, which Microsoft licenses separately under the
Microsoft Software License Terms (shown in full in Inkwell's Settings > About). By installing or
using Inkwell on Windows, you agree to those terms for those components.

**Why the warnings?** Windows warns about any app that is not signed with a code-signing
certificate it recognises. Signing the Windows build is planned; until then, the SHA-256 in step 2
is the check.

---

Notes for the release-prep step (not homepage text):

- The same terms sentence is in each release's notes (win-release.yml), on the installer's splash
  (`$SplashTerms` in windows/scripts/pack.ps1) and in Settings > About and the first run's terms
  step (`Notices.WindowsAppSdkTerms`); keep the four in step if one changes.
- Smart App Control: Microsoft's own answer is that no single app can be allowed past it, only the
  whole feature turned off, and that it cannot then go back to evaluation mode without resetting
  Windows ([Smart App Control FAQ](https://support.microsoft.com/en-us/windows/security/threat-malware-protection/smart-app-control-frequently-asked-questions)).
  Hence the advice to wait rather than to turn it off.
- `sha256sum -c Inkwell_X.Y.Z_windows-sha256.txt` (Git Bash, WSL) checks all three Windows files at
  once, for anyone who prefers it.
