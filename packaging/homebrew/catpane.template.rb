cask "catpane" do
  version "__VERSION__"

  on_arm do
    sha256 "__SHA_ARM64__"

    url "https://github.com/__REPOSITORY__/releases/download/v#{version}/CatPane-v#{version}-macos-arm64.zip"
  end

  on_intel do
    sha256 "__SHA_X86_64__"

    url "https://github.com/__REPOSITORY__/releases/download/v#{version}/CatPane-v#{version}-macos-x86_64.zip"
  end

  name "CatPane"
  desc "Native desktop logcat viewer with split panes"
  homepage "https://github.com/__REPOSITORY__"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on formula: "android-platform-tools"

  app "CatPane.app"

  zap trash: [
    "~/.config/catpane/session.json",
    "~/.config/catpane/tag_history.txt",
  ]
end
