# furigana4epub

Add hiragana furigana to Japanese EPUB 3 books using Sudachi.

## CLI usage

Install the CLI from the repository root, then download the Sudachi Full V1 dictionary and convert a book.

```sh
cargo install --path cli --locked
furigana4epub update-dict
furigana4epub book.epub
```

To run the CLI directly from the repository root, use `cargo run`. Put CLI options and paths after `--`.

```sh
cargo run --locked -p cli -- update-dict
cargo run --locked -p cli -- book.epub
```

The CLI writes `book.furigana.epub` beside `book.epub`. Pass a second path to choose a different destination. The destination must be a new file.

```sh
furigana4epub --highlight-color '#2563eb' book.epub annotated.epub
```

After a successful conversion, the CLI prints the output path, elapsed time, and settings used.

Rerun the dictionary update command to refresh it. The CLI and GUI share the copy in the `furigana4epub` folder of your local user data directory.

To remove the shared dictionary, run `furigana4epub remove-dict`. From the repository root, you can also run `cargo run --locked -p cli -- remove-dict`.

### Options

| Option                    | Effect                                                                                               |
| ------------------------- | ---------------------------------------------------------------------------------------------------- |
| `--dictionary <PATH>`     | Use a local Sudachi V1 dictionary in place of the shared default.                                    |
| `--override-existing`     | Replace existing ruby when Sudachi can read its entire base. By default, existing ruby is preserved. |
| `--highlight-injected`    | Highlight the generated furigana in the default color, `#c2410c`.                                    |
| `--highlight-color <HEX>` | Highlight the generated furigana in a CSS hex color such as `#2563eb`.                               |
| `--new-identifier`        | Generate a new primary EPUB identifier.                                                              |
| `--identifier <VALUE>`    | Set the primary EPUB identifier explicitly. Takes precedence over `--new-identifier`.                |
| `-h`, `--help`            | Show command-line help.                                                                              |

The original identifier is preserved by default. Changing it requires an EPUB without encrypted or obfuscated resources.

## GUI usage

Launch the app and click **Update Dictionary** to download or refresh the shared dictionary. Choose an EPUB and set your options. Click **Convert EPUB**, then choose where to save the converted book.

### Download from GitHub Releases

Open the [latest release](https://github.com/nohackjustnoobb/furigana4epub/releases/latest). Under **Assets**, choose the archive for your operating system and processor (`x64` or `arm64`). Extract the archive. On macOS and Linux, run `./furigana4epub` from the extracted folder in your terminal. On Windows, open `furigana4epub.exe`.

Alpha, beta, and RC builds are listed under [all releases](https://github.com/nohackjustnoobb/furigana4epub/releases).

Linux builds use Ubuntu 26.04 and require compatible glibc and desktop libraries, plus a working Vulkan driver.

### Run or build from source

Use Rust 1.98 or newer and install the [GPUI Kit prerequisites for your operating system](https://gpui-kit.com/docs/installation/#platform-requirements).

Run the GUI from the repository root.

```sh
cargo run --locked -p gui
```

To build the executable, run `cargo build --release --locked -p gui`. Find it at `target/release/gui` (`target/release/gui.exe` on Windows).
