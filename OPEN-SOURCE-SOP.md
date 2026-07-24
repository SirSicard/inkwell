# Open Source Quality SOP for Inkwell

Based on research of top repos: Tauri (95k stars), RustDesk (82k), Jan (28k), Stirling-PDF (55k), Clay (35k).

---

## Required Files (The Bare Minimum for "Quality")

### 1. README.md ✅ (exists, needs polish)
**What the best repos do:**
- Hero banner/logo at top (centered, clean)
- Badges row: build status, license, release version, downloads, Discord/community
- One-sentence description
- Screenshot or GIF demo (this is HUGE for desktop apps, RustDesk/Jan both lead with screenshots)
- Feature bullets (short, scannable)
- Install section per platform with copy-paste commands
- "Quick Start" (3-5 steps max)
- Tech stack (brief)
- Build from source instructions
- Contributing link
- License

**Inkwell gap:** No badges, no screenshot/GIF, no build-from-source section is thin. These are the quick wins.

### 2. LICENSE ✅ (just created)

### 3. CONTRIBUTING.md ❌ (missing)
Every serious repo has this. Template:

```
- Welcome message (1-2 sentences)
- How to report bugs (use issue templates)
- How to suggest features
- How to submit PRs (fork > branch > PR)
- Code style / commit message conventions
- "Claim the issue first" rule (prevents duplicate work)
- Dev setup instructions (clone, install deps, run)
- Link to Code of Conduct
```

Keep it short. Stirling-PDF's is ~60 lines and covers everything.

### 4. CODE_OF_CONDUCT.md ❌ (missing)
Use the Contributor Covenant (standard). GitHub can auto-generate this.
Not exciting but signals "we take this seriously." Every top repo has it.

### 5. SECURITY.md ❌ (missing)
Short file explaining how to report security vulnerabilities privately.
Template:
```
# Security Policy

## Reporting a Vulnerability

Please report security vulnerabilities by emailing [your email].
Do NOT open a public issue for security vulnerabilities.

We will acknowledge receipt within 48 hours and provide a timeline for a fix.
```

### 6. CHANGELOG.md ❌ (missing)
Even if it's just one entry for v0.1.0. Shows the project has a pulse.
Format: Keep a Changelog (keepachangelog.com).

### 7. Issue Templates ❌ (missing)
`.github/ISSUE_TEMPLATE/bug_report.md` and `feature_request.md`
GitHub auto-populates these when someone opens an issue. Reduces noise massively.

### 8. PR Template ❌ (missing)
`.github/pull_request_template.md`
Checkbox list: tests pass, docs updated, screenshots if UI change.

---

## README Upgrade Checklist

- [ ] Add centered logo/wordmark at top
- [ ] Add badge row: ![Build](CI badge) ![License](MIT) ![Release](v0.1.0) ![Downloads](shield)
- [ ] Add screenshot or GIF (record a 10-second demo of hotkey > speak > paste)
- [ ] Expand "Development" section with full build-from-source steps (prereqs, clone, install, run)
- [ ] Add "Contributing" section linking to CONTRIBUTING.md
- [ ] Add "Community" section (even if it's just GitHub Discussions for now)

---

## GitHub Repo Settings

- [ ] **Description:** "Local-first speech-to-text for desktop. Private. Fast. Free." (short, punchy)
- [ ] **Topics:** `speech-to-text`, `stt`, `dictation`, `tauri`, `rust`, `desktop-app`, `privacy`, `local-first`, `voice`, `transcription`
- [ ] **Website:** https://inkwell-homepage.vercel.app
- [ ] **Discussions:** Enable (free support channel, reduces issue noise)
- [ ] **Releases:** Already done
- [ ] **Social preview image:** (1280x640 OG image for link previews)

---

## Nice-to-Have (Do Later)

- **ARCHITECTURE.md**: How the codebase fits together. Tauri has this. Helpful for contributors.
- **Translated READMEs**: RustDesk has 25+ languages. Overkill for now but signals global ambition.
- **GitHub Actions badge in README**: Shows builds pass. Trust signal.
- **GitHub Discussions categories**: Q&A, Feature Requests, Show & Tell
- **DCO sign-off policy**: RustDesk requires this. Overkill for a solo project but good if you get corporate contributors later.

---

## Priority Order (What to Do First)

1. **Screenshot/GIF in README** (biggest impact, people scroll past text)
2. **Badges in README** (trust signals)
3. **CONTRIBUTING.md** (signals "contributions welcome")
4. **Issue templates** (reduces noise from day one)
5. **CODE_OF_CONDUCT.md** (auto-generate, 30 seconds)
6. **SECURITY.md** (30 seconds to write)
7. **CHANGELOG.md** (5 minutes)
8. **PR template** (5 minutes)
9. **Social preview image** (for link shares)
10. **GitHub topics + description** (30 seconds)

---

## What NOT to Do

- Don't over-engineer governance for a v0.1.0 project
- Don't add a CLA (Contributor License Agreement) unless you plan to dual-license later
- Don't create 15 issue labels before you have 15 issues
- Don't write a 500-line CONTRIBUTING.md. Short and welcoming beats thorough and intimidating.
- Don't add automated bots (stale bot, welcome bot) until you actually have traffic

---

*This SOP is a one-time setup checklist. Once done, maintain the CHANGELOG and respond to issues/PRs. That's it.*
