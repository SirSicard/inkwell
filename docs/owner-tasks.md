# Owner tasks

Step-by-step for the four things waiting on you. Written against **v0.2.1**,
which is what is installed; where a later build differs, it says so.

---

## 1. Record an accuracy corpus

**Why:** three Parakeet builds are installable and nobody knows which hears
*you* best. The comparison tool exists and has nothing to run on. Guessing from
one good sentence is how the last round of "it feels better" happened.

**Time:** about 15 minutes, most of it typing.

### Turn recording on

1. Open Inkwell, go to **General**.
2. Switch **Advanced Mode** on if it is off.
3. Switch **Save Debug Audio** on.

> In v0.2.1 this lives in General. A later build moves it to its own
> **Troubleshooting** tab, along with a button that opens the recordings folder.

### Dictate

Dictate the sentences below **as normally as you can**: same distance from the
mic, same pace, same room you actually work in. A corpus recorded carefully in a
silent room measures a situation you are never in.

Cover all five kinds. They each test a different part of the pipeline:

| # | What to say | What it tests |
|---|---|---|
| 1 | A sentence with **Inkwell, Claude and Vercel** in it | Hotword biasing, the fix that shipped in 0.2.1 |
| 2 | A short one, 3 to 5 words | Word edges: the pre-roll and release-tail capture |
| 3 | A long one, **over 30 seconds without stopping** | Chunk seams at the 60s cut, and the merge |
| 4 | One with a **deliberate 2 second pause** in the middle | VAD trimming pauses instead of gating them |
| 5 | One said naturally **with "um" and a restart** in it | Cleanup, and whether restarts confuse the decoder |

Do two or three of each. Ten to fifteen recordings total is plenty; a hundred
would not tell you more, because the differences between these models are not
subtle.

### Write down what you actually said

Recordings land in **`~/Documents/Inkwell Debug Audio/`** as `take-0001.wav`,
`take-0002.wav` and so on, in the order you made them.

For each one, create a text file with the same name and a `.txt` extension:

```
~/Documents/Inkwell Debug Audio/
  take-0001.wav
  take-0001.txt      <- what you actually said
  take-0002.wav
  take-0002.txt
```

**Write it verbatim, not tidied.** If you said "um" or restarted a word, write
that. The tool scores the recogniser's raw output, before cleanup runs, so a
tidied reference would count the recogniser as wrong for hearing you correctly.

Case and punctuation do not matter and are ignored when scoring.

Any recording without a `.txt` is still transcribed so you can read it, but it
does not count toward the score. So if one take is unusable, skip its `.txt`
rather than deleting the wav.

### Turn it back off

Return to **General** and switch **Save Debug Audio** off. It writes your voice
to disk while it is on. Delete the folder when the comparison is done.

### Then tell me

I will download Parakeet V2 fp16 and V3 full precision, run all three over your
corpus, and report word error rate per model. That decides the default, with a
number instead of an impression.

---

## 2. Buy a domain

**Why:** one purchase upgrades three things at once, the homepage, the updater
endpoint and the link previews. `getinkwell.vercel.app` works today and carries
no personal name, so this is an upgrade rather than a fix.

**Cost:** roughly EUR 10 to 15 a year for a `.com`.

### Availability, checked 2026-07-31

Taken: `getinkwell.com`, `inkwell.app`, `useinkwell.com`, `inkwell.sh`,
`tryinkwell.com`, `inkwell.ink`, `getinkwell.app`, `inkwell-app.com`,
`inkwell.press`.

Apparently free (no nameservers, which is a strong hint but not proof, so
confirm at the registrar):

- **`inkwell.tools`** — shortest, and the TLD says what it is
- `writeinkwell.com`
- `inkwelldictation.com`
- `hey-inkwell.com`

"Inkwell" is a common English word, so the good short domains went long ago.
`.tools` or a two-word `.com` is the realistic tier.

### Steps

1. Buy at any registrar. Cloudflare Registrar sells at cost with no renewal
   markup and no upsells; Namecheap and Porkbun are also fine. Avoid GoDaddy's
   introductory pricing, which renews high.
2. Turn on WHOIS privacy. It is free at all three above and keeps your home
   address out of a public database.
3. Tell me the domain. I will point it at Vercel, update `SITE_URL`, move the
   updater endpoint off `workers.dev` and re-verify an update actually installs.

Do **not** buy a `.io` if you can avoid it: the TLD's long-term status has been
in question since the territory's political change, and a dictation app should
not have a domain with an asterisk on it.

---

## 3. Windows 11 test environment

**Why:** Windows builds and ships in every release and has never once been run.
The keyring backend, the paste path, per-app detection and voice editing's copy
keystroke are all untested guesses on that platform.

**Cost:** nothing, if you use the free route.

### The licensing question, answered

Windows 11 installs and runs **without a product key**, indefinitely and
legally. You get a watermark in the corner and the personalisation settings are
locked. Everything else, including everything Inkwell touches, works normally.
For testing, that is enough. Do not buy a key for this.

### Route A: UTM, free

1. Install UTM: `brew install --cask utm`, or from mac.getutm.app.
2. Install CrystalFetch (free, in the Mac App Store). It downloads an official
   Windows 11 ARM image straight from Microsoft and builds an ISO.
3. In UTM, create a Virtualize > Windows machine from that ISO. Give it 8 GB of
   RAM and 4 cores.
4. Skip the product key when asked. Choose "I don't have a product key".

### Route B: Parallels, about EUR 100 a year

Faster and less fiddly: Parallels downloads and installs Windows 11 for you in
one step, and its clipboard and display integration are better. Worth it only if
you will use a Windows VM for other things too.

### The catch, either way

Your Mac is Apple Silicon, so the VM runs **Windows on ARM**, and Inkwell's
release build is x64. It will run under Microsoft's x64 emulation. That is fine
for what we need to test, which is whether things *work*: the hotkey, the paste,
the keyring, the overlay.

It is useless for judging speed. Do not conclude anything about transcription
being slow on Windows from a VM; emulated inference is several times slower than
native and tells you nothing about a real Windows PC.

### Then tell me

I will write a QA checklist and we will work through it, or you run it and
report what breaks.

---

## 4. Windows code signing

**Why:** an unsigned Windows installer makes SmartScreen show "Windows protected
your PC" with a "Don't run" button. It is a harder stop than macOS Gatekeeper,
because the way past it is not obvious.

**Cost:** about USD 10 a month, or about USD 200 to 400 a year for the
alternative.

### Check eligibility first

Since June 2023, code signing keys must live on certified hardware, which killed
the cheap certificate market. Two routes remain:

**Azure Trusted Signing, ~USD 10/month.** By far the cheapest. Requires an
identity that Microsoft can verify, and the rules for individual developers have
changed more than once, so **check current eligibility before planning around
it**:

1. Sign in at portal.azure.com (a free Azure account is enough to look).
2. Search for **Trusted Signing** and start creating an account.
3. It will state the current identity-validation requirements. Read them before
   paying for anything.

**OV certificate, ~USD 200 to 400/year**, from Sectigo, DigiCert or SSL.com.
Works for individuals, but the key must be on a hardware token they ship you, or
in a cloud HSM. Still triggers SmartScreen until the certificate builds
reputation across enough downloads.

### Worth knowing before you spend

An **EV** certificate (~USD 400 to 700/year) is the only thing that grants
instant SmartScreen reputation. Everything cheaper still shows a warning at
first and earns trust over time.

With the current download numbers, signing Windows buys very little today. This
is the least urgent of the four, and it is reasonable to leave the Windows build
unsigned until somebody actually reports being blocked by it.

### Then tell me

Whichever route, the certificate goes into GitHub secrets and `build.yml` picks
it up the same way the Apple secrets do. I will wire it.
