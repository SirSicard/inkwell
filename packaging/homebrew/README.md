# Homebrew cask

`inkwell.rb` installs the released macOS build.

## Publishing it

Homebrew casks live in a tap repository, not in the app repository. To publish:

1. Create a public GitHub repo named **`homebrew-tap`** under the same account.
2. Copy `inkwell.rb` into it as `Casks/inkwell.rb`.
3. Users install with:

   ```
   brew install --cask sirsicard/tap/inkwell
   ```

Nothing installs it from this directory: a cask sitting in the app repo is a
draft, not a distribution channel.

## Every release

The checksum pins one exact file, so it must be regenerated:

```
shasum -a 256 Inkwell_<version>_aarch64.dmg
```

Then bump `version` and `sha256` together. Homebrew refuses to install on a
mismatch, which is deliberate: while the app is unsigned, the checksum is the
only thing standing between the user and a substituted download.

## Checked before committing

`brew audit --cask --strict` passes. It runs against a tap, not a path, so
validating a change means cloning this file into a scratch tap first. The audit
already caught one real mistake here (a redundant `verified:` on a url whose
domain matches the homepage).

## Remove the quarantine workaround after signing

`postflight` clears the Gatekeeper quarantine flag, which is only acceptable
because the app is not yet notarized. When Developer ID signing lands, delete
that block and the caveat that explains it.
