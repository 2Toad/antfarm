# Rendered by the antfarm release workflow from packaging/homebrew in 2Toad/antfarm.
# Edit the template there, not this file.
class Antfarm < Formula
  desc "Live view of what Claude Code agents on your machine are doing"
  homepage "https://github.com/2Toad/antfarm"
  url "https://github.com/2Toad/antfarm/releases/download/v@VERSION@/antfarm-@VERSION@-macos-universal.tar.gz"
  sha256 "@CLI_SHA256@"
  license any_of: ["MIT", "Apache-2.0"]

  depends_on :macos

  def install
    bin.install "antfarm"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/antfarm --version")
  end
end
