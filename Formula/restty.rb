class Restty < Formula
  desc "Responsive, safety-first terminal output formatter"
  homepage "https://github.com/restty-cli/restty"
  head "https://github.com/restty-cli/restty.git", branch: "main"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "restty ", shell_output("#{bin}/restty --version")
    assert_match "__restty_run", shell_output("#{bin}/restty init bash")
  end
end
