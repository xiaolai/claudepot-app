cask "claudepot" do
  version "0.0.2"

  on_arm do
    sha256 "REPLACE_ME_DMG_AARCH64_SHA256"

    url "https://github.com/xiaolai/com.claudepot.app/releases/download/v#{version}/Claudepot-aarch64.dmg"

    binary "#{appdir}/Claudepot.app/Contents/MacOS/claudepot-cli-aarch64-apple-darwin",
           target: "claudepot"
  end
  on_intel do
    sha256 "REPLACE_ME_DMG_X86_64_SHA256"

    url "https://github.com/xiaolai/com.claudepot.app/releases/download/v#{version}/Claudepot-x86_64.dmg"

    binary "#{appdir}/Claudepot.app/Contents/MacOS/claudepot-cli-x86_64-apple-darwin",
           target: "claudepot"
  end

  name "Claudepot"
  desc "Multi-account Claude Code / Claude Desktop switcher"
  homepage "https://github.com/xiaolai/com.claudepot.app"

  livecheck do
    url :homepage
    strategy :github_latest
  end

  depends_on macos: ">= :catalina"

  app "Claudepot.app"

  zap trash: [
    "~/.claudepot",
    "~/Library/Caches/com.claudepot.app",
    "~/Library/Preferences/com.claudepot.app.plist",
    "~/Library/Saved Application State/com.claudepot.app.savedState",
    "~/Library/WebKit/com.claudepot.app",
  ]
end
