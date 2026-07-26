# Buy Me a Coffee page copy

Paste-ready text for the Inkwell donation page. Written to match the site's
voice: plain, specific, no guilt. Every claim here is true of the shipped app.
If a feature changes, change this too.

**Before publishing:** create the account yourself, then put the real handle in
`homepage/lib/constants.ts` (`DONATION_URL`) and in the app's
`src/constants.ts`. Both currently point at `buymeacoffee.com/REPLACE_ME`.

---

## Page name

```
Inkwell
```

## What people are buying

BMC asks what a supporter is buying. Keep it literal:

```
a coffee
```

## Short bio (the line under your name)

```
I build Inkwell: free, open-source dictation that runs entirely on your own machine. No account, no subscription, no telemetry.
```

Alternates, if you want a different emphasis:

```
Hold a key, speak, let go. Inkwell turns your voice into text on your own machine, and never sends it anywhere.
```

```
Free local dictation for macOS and Windows. Your voice never leaves your computer, and it never will.
```

---

## About section

```
Inkwell is a dictation app for macOS and Windows. You hold a hotkey, speak, let
go, and the text lands in whatever app you were already typing in.

The whole point is that it runs on your machine. Speech recognition happens
locally through a model you download once. Nothing is uploaded, there is no
account to make, and there is no telemetry of any kind. Your voice is not a
training set, a subscription, or a growth metric.

It is free. Not free-for-now, not free-until-the-seed-round: there is no paid
tier, no licence key, and no feature sitting behind a paywall. The source is MIT
licensed and on GitHub, so if I ever stopped maintaining it you could keep it
running yourself.

Why the tip jar, then? Two things about shipping a desktop app cost real money no
matter how the code is licensed. Signing and notarising a macOS build needs a
paid Apple developer account, and until that is in place every download greets
you with a scary warning and a workaround. A domain to host it on is the other.
Coffee covers those.

To be clear about what a tip buys you: nothing. You already have the whole app.
Tipping does not unlock anything, does not move your bug up a queue, and does not
get you a different version. It just means the next person's download is less
frightening.

And honestly, the unpaid help is worth more than the coffee. Tell me what broke
on your hardware, which model got your accent wrong, or which app it refused to
paste into. That is the stuff I cannot get any other way.

Mattias
```

---

## Thank-you message (shown after a donation)

```
Thank you, that genuinely helps.

If you have thirty seconds: tell me what you use Inkwell for and what has annoyed
you about it. Bug reports and rough edges are the most useful thing you can send
me, and they cost nothing.
```

---

## Notes on what this copy deliberately avoids

- **No user counts or social proof.** The published build has a couple of dozen
  downloads. Claiming momentum it does not have would be the one thing that makes
  the rest of the page untrustworthy.
- **No roadmap promises.** Live-as-you-speak transcription does not exist in
  Inkwell, so it is not hinted at here.
- **No "support development or it dies" framing.** The app is MIT and finished
  enough to use; implying otherwise would be a manufactured deadline.
- **Signing and domain are named as the actual costs** rather than a vague
  "supports development", because they are specific, verifiable, and currently
  true: the builds really are unsigned today.
