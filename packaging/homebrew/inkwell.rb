# Homebrew cask for Inkwell.
#
# Not yet published. To publish, create a tap repository named
# `homebrew-tap` under the same GitHub account and copy this file to
# `Casks/inkwell.rb` in it. Users then install with:
#
#     brew install --cask sirsicard/tap/inkwell
#
# `sha256` must be regenerated for every release:
#
#     shasum -a 256 Inkwell_<version>_aarch64.dmg
#
# The alternative is `sha256 :no_check`, which tells Homebrew to install
# whatever the URL happens to serve. Worth keeping even now that the dmg is
# signed and notarised: Gatekeeper tells you the download came from this
# Developer ID, and the checksum tells you it is the exact build this cask was
# written against. They answer different questions.
cask "inkwell" do
  version "0.2.9"
  sha256 "320dc539a120ad3afac1da69d5c61182040687d55cbf66aadc86d967ccc854c2"

  # No `verified:` here: it is only for urls whose domain differs from the
  # homepage, and brew audit rejects it as redundant when they match.
  url "https://github.com/SirSicard/inkwell/releases/download/v#{version}/Inkwell_#{version}_aarch64.dmg"
  name "Inkwell"
  desc "Local-first dictation that types where you type"
  homepage "https://github.com/SirSicard/inkwell"

  livecheck do
    url :url
    strategy :github_latest
  end

  # Apple Silicon only for now: the release builds an aarch64 dmg, and the
  # Intel target produces a separate artifact that would need its own url/sha
  # pair here. Declaring support for a slice we do not ship would fail at
  # install time rather than at `brew info`.
  depends_on arch: :arm64
  depends_on macos: ">= :big_sur"

  app "Inkwell.app"

  uninstall quit: "com.inkwell.app"

  # Models are large downloads the user chose to make and may want to keep
  # across a reinstall, so they are only removed by `brew uninstall --zap`.
  zap trash: [
    "~/Library/Application Support/com.inkwell.app",
    "~/Library/Logs/com.inkwell.app",
    "~/Library/Saved Application State/com.inkwell.app.savedState",
  ]

  caveats <<~EOS
    On first launch, grant:
      Microphone     (System Settings > Privacy & Security > Microphone)
      Accessibility  (System Settings > Privacy & Security > Accessibility)

    Accessibility is what lets Inkwell paste into the app you are typing in.
    Without it, dictation transcribes but nothing appears.
  EOS
end
