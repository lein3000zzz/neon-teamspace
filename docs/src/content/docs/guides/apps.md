---
title: Apps
description: The desktop and mobile apps, which open your own Neon Teamspace server in a window of their own. Installing them, connecting and switching servers, what works offline, their security model, and building them from source.
---

The apps for macOS, Windows, Linux, Android and iOS open the Neon Teamspace server you choose.
Each one is a thin shell around the platform's own web view. The only screen the app brings
is the server picker: everything after it, signing in included, is the server's own web
interface, the same one a browser shows. An app is never older or newer than the server it
shows, and the server needs no setting for it.

## Installing

Each [release](https://github.com/lein3000zzz/neon-teamspace/releases) has the desktop
installers and the Android APK attached. The iPhone and iPad app goes out through TestFlight.

| Platform | File | Notes |
|---|---|---|
| macOS 11 or later | `Neon-Teamspace_<version>_aarch64.dmg` for Apple silicon, `_x64.dmg` for Intel | Open the disk image and drag the app to Applications. |
| Windows 10 or later | `Neon-Teamspace_<version>_x64-setup.exe` | The installer adds Microsoft's WebView2 runtime when it is missing. |
| Linux with glibc 2.35 or later, such as Ubuntu 22.04, Debian 12 or Fedora 36 | `.deb`, `.rpm` or `.AppImage` | The `.deb` and `.rpm` depend on WebKitGTK 4.1, which the package manager installs. The AppImage brings its own, from Ubuntu 22.04, where the apps are built. Make an AppImage executable with `chmod +x` before running it. |
| Android 7 or later | `Neon-Teamspace_<version>_android.apk` | Allow your browser or file manager to install apps, then open the file. |
| iOS and iPadOS 15 or later | TestFlight | Whoever publishes the app sends the invitation. |

A release candidate, such as `v0.2.0-rc.1`, is marked as a pre-release, and its files carry
the candidate's version in their names, as `Neon-Teamspace_0.2.0-rc.1_x64-setup.exe`. The
apps in them are those of the release the candidate rehearses, and report its version.

The macOS and Windows apps are not signed with a developer certificate, so the system warns
the first time one opens. On macOS, try to open the app once, then choose **Open Anyway** for
it under **System Settings > Privacy & Security** and confirm. On macOS 14 and earlier,
Control-click the app in Finder and choose **Open** instead. On Windows, choose **More info**
and **Run anyway** in the SmartScreen dialog.

A release built without the project's Android signing key has
`Neon-Teamspace_<version>_android-unsigned.apk` instead. Android refuses to install an unsigned
APK. Sign it with your own key using `apksigner` from the Android SDK, or build a debug APK as
[below](#building-from-source).

### Checking a download

As the apps are not signed for macOS and Windows, check a download before you override the
system's warning. Each release attaches `SHA256SUMS`, the SHA-256 checksums of everything it
attaches. In the folder with the download and `SHA256SUMS`:

```sh
sha256sum --check --ignore-missing SHA256SUMS            # Linux
shasum -a 256 --check --ignore-missing SHA256SUMS        # macOS
```

On Windows, `Get-FileHash Neon-Teamspace_<version>_x64-setup.exe` in PowerShell prints the
checksum to compare with the file's line.

The installers and the APK also carry build provenance: a signed record, kept by GitHub, of the
release workflow's run and the commit that built the file. With the
[GitHub CLI](https://cli.github.com):

```sh
gh attestation verify Neon-Teamspace_<version>_aarch64.dmg --repo lein3000zzz/neon-teamspace
```

It fails for a file that the repository's release workflow did not build. Releases made while
the repository was private have checksums but no provenance.

## Connecting to a server

On first launch the app asks for the server's address. Enter the one you open Neon
Teamspace at in a browser, such as `neon.example.com`. The app

1. adds `https://` when the address has no scheme, keeps only the origin, and drops a path,
   a query, the default port and a trailing dot;
2. asks the server for `GET /api/v1/meta`, waiting up to 10 seconds and reading at most
   64 KiB, without following redirects;
3. checks that the answer is JSON from a Neon Teamspace server, with its instance name and
   a version number;
4. saves the server, and opens its root page, where the server's own sign-in page takes over.

The app refuses an address that fails a step and says why:

| Message | What to do |
|---|---|
| Only https:// addresses can be used… | The address starts with another scheme, such as `ftp://`. |
| Leave the user name and password out of the address… | Remove `name:password@`. You sign in on the server's page. |
| … needs https://. Plain http:// is only for a server on this device or your own network… | See [Plain http](#plain-http) below. |
| Could not reach … | The server is down, the name does not resolve, or the connection failed. The message ends with the system's reason, or for https what was wrong in plain words, such as an expired certificate, one the device does not trust, or a server that answered on plain http. The log keeps the technical cause. |
| … did not answer within 10 seconds. | The server or the network is too slow, or a firewall drops the connection. |
| … sends its visitors to …. Enter that address instead. | The server redirects. Use the address it names, which is the one its administrator set as `server.public_url`. |
| … has no Neon Teamspace API: /api/v1/meta answered 404. | The address is another site, or a proxy in front of the server does not pass `/api` on. |
| … answered with something other than JSON… | The address serves web pages but no Neon Teamspace API. |
| … answered with JSON that is not a Neon Teamspace server's… | Another product answers at that path, or the server is a development build older than the apps. |

Use the address the server's administrator gives out. The server accepts changes only from
pages on its own origin, `server.public_url`, so the app must load the server at that origin
too: an IP address in place of the domain loads the pages, but saving fails.

### Plain http

An `http://` address is taken only when the host is this device or on a private network. That
means a loopback, private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), shared
(`100.64.0.0/10`, as Tailscale uses) or link-local address, an IPv6 unique local or link-local
address, or a name under `localhost`, `.local`, `.home.arpa` or `.internal`. The app decides
from the address as typed and never resolves the name. For a server elsewhere it insists on
`https://`.

Before saving an `http://` server the app warns that anyone on the network can read what you
send it, your password included, and connects only once you choose **Connect anyway**. The
list marks the server **Not encrypted**.

Over plain http the server names its session cookie `neon_session` rather than
`__Host-neon_session`, because browsers only keep `__Host-` cookies that come over https.
The app's web view keeps it like a browser does, and signing in, real-time editing and files
work the same. The difference is that the cookie, like everything else, crosses the network
in the clear.

## Switching servers

Once a server is saved, the picker lists the saved servers with their names, addresses and
versions. Choose one to open it. The app checks it again first, and shows the reason on its
row if the check fails. **Rename** gives a server a name of your own, and an empty name goes
back to the instance's. **Remove** forgets the address. It does not sign you out: sign out in
the server's interface first to end the session. **Add a server** connects to another one.

As it starts, the app opens the server you used last. When that server cannot be reached, the
picker shows the list and the reason.

To get back to the picker from a server:

- **macOS**: **Neon Teamspace > Switch Server…** in the menu bar.
- **Windows and Linux**: **File > Switch Server…**.
- **Android**: the system's back gesture or button. It goes back through the pages you
  visited, and from the first of them to the picker.
- **iOS and iPadOS**: swipe from the left edge of the screen, which goes back the same way.

On mobile the app draws nothing over the server's pages, so the way back is the platform's own
back gesture. The desktop menu also has **View > Back**, **Forward** and **Reload**, which a
window without a toolbar has nowhere else; **Back** from the server's first page shows the
picker too. Each way loads the picker afresh, after a blank page, as the
[security model](#security-model) explains.

Each server keeps its own sign-in. The web view stores cookies per origin, as a browser does,
so switching between servers does not sign you out of either.

## Links, files and offline use

A link to another site opens in your default browser, and `mailto:` and `tel:` links in the
apps that handle them. A page of the same server that the interface opens in a new window, such
as a file, opens in the app's window instead, on every platform. **Back** returns to where you
were.

The app has no way to tell a click from a script, so it opens something outside at most once a
second, not the same address twice within ten seconds, and nothing in the first second after a
page starts loading. A page cannot open a flood of browser tabs that way, but a link chosen
right after a page appears may need a second try. On macOS, iOS and Linux the app shows no
frame from another site: such a frame is opened in the browser like a link, within the same
limits. On Windows and Android it loads inside the page, as in a browser. The server's own
Content-Security-Policy allows frames from the server alone, so on a standard install the
interface has none from elsewhere.

Choosing a file to upload works on every platform, and so does dropping files on a page on
the desktop. Downloads from the server land in:

- **Desktop**: your Downloads folder. The app shows the file in its folder when the download
  ends.
- **Android**: the Downloads folder, through the system's download manager and its
  notification. Android 9 and older keep the file in the app's own folder, which the
  notification opens. A file the page makes itself, such as a workspace export, cannot be saved
  in the Android app; use a browser for those.
- **iOS and iPadOS**: the app's folder in the Files app, under **On My iPhone** or
  **On My iPad**.

Nothing beyond the picker works offline. The app keeps the list of servers and the picker's
theme on the device; everything else comes from the server each time, as it does in a
browser.

The picker follows the system's light or dark mode, and its **Theme** choice offers the web
app's palettes: Nothing, Soft and Warm. The server's interface keeps your own appearance
settings, as it does in a browser.

## Security model

**Origin lock.** The web view loads only the app's own origin, which serves the bundled
picker, and the origins of the servers you opened since the app started. The window shows the
one in use; the others stay open to the lock because the web view's history keeps their pages,
and Android's back gesture walks that history without asking the app. Removing a server, or
quitting the app, closes the lock to it. Two URLs share an origin when their scheme, host and
port are the same after normalising, as in a browser:

- `https://neon.example.com:443/w/acme` is the server `https://neon.example.com`;
- `http://neon.example.com`, `https://neon.example.com:8443`, `https://www.neon.example.com`
  and `https://neon.example.com.` (with a trailing dot) are other origins;
- in `https://neon.example.com@evil.example/` the host is `evil.example`; the part before
  the `@` is a user name, which counts for nothing;
- an international name matches its ASCII form, so `bücher.example` and
  `xn--bcher-kva.example` are the same host, while a lookalike letter from another alphabet
  makes another name.

Any navigation to another `http` or `https` origin is cancelled and opens in the system's
browser instead, within the limits [above](#links-files-and-offline-use). That covers links,
`window.open`, redirects, and sign-in or consent pages that leave the server. Other schemes,
such as `file:`, `data:` or `javascript:`, are refused. Downloads are taken only from the
server's origin. A page that loads without asking the lock, as a form that a server's page
posts to another site does on Android, is left at once: it opens in the browser, and the app
goes back to the server's last page.

A server's page never takes the web view to the app's own origin. When one tries, with a link,
a redirect or the history, the app cancels it and shows the picker itself: it loads a blank
page, which replaces the server's, and the picker only once that has loaded. Only the picker
can call the app's commands, and on Linux the web view names the page a call comes from by
the address it is loading, so a server's page that was still there while the picker loaded
could pass as the picker. The commands also run only while the page the window last loaded is
the app's own. On Windows, whose web view names each call's own page, the picker loads
without the blank page.

**Capabilities.** Tauri lets a page call the app's native side only through the capabilities
granted to it. The bundled picker has one, which allows the five commands it uses: list,
connect, open, rename and remove servers. It grants no plugin, so the picker can neither open
URLs nor write logs itself; the app's Rust side does both. The server's pages are on another
origin and get no capability at all: every call to the native side from them is refused, and
they run as they would in a browser.

**The picker's policy.** The picker's Content-Security-Policy allows only the app's own
scripts, styles and images, no frames, and network access only to the app itself. It loads
nothing from the network: the app's Rust side checks servers, not the picker.

**The check.** Checking an address connects from this device to the host and port you enter,
which may be on your local network, as a browser does when you type an address. Only the
picker can start a check, and its message says whether anything answered there, such as
"Connection refused", so the picker tells you a little about the network the device is on.

**What the app keeps.** `servers.json` in the app's data directory holds each server's origin,
the name you gave it, and its instance name and version from the last check. Nothing in it is
secret. Sessions are the server's cookies, which the web view keeps with its other website
data. The Android app turns off Android's backups, so its sessions go to no cloud backup and
no new phone; where a device moves an app's data to a new one all the same, only
`servers.json` goes.

| Platform | Data directory | Log |
|---|---|---|
| macOS | `~/Library/Application Support/io.github.lein3000zzz.neonteamspace` | `~/Library/Logs/io.github.lein3000zzz.neonteamspace` |
| Windows | `%APPDATA%\io.github.lein3000zzz.neonteamspace` | `%LOCALAPPDATA%\io.github.lein3000zzz.neonteamspace\logs` |
| Linux | `~/.local/share/io.github.lein3000zzz.neonteamspace` | `~/.local/share/io.github.lein3000zzz.neonteamspace/logs` |
| Android | the app's private storage | `adb logcat` |
| iOS | the app's private storage | the device's console |

The log records each check, and each navigation the origin lock decides, with the origin it
went to and never the path or query, which can carry a page's name or a token. So does a line
about an address the system could not open.

**Certificates.** The check trusts the certificates the system trusts, as the web view does.
On Android it trusts Mozilla's root certificates, which Android's own store follows. A server
with a certificate from a private authority works on the desktop and iOS once the authority is
in the system's trust store; Android apps do not trust authorities a user adds, so such a
server needs a publicly trusted certificate there.

## Building from source

The apps' project is `apps/shell/` in the repository: the picker in TypeScript, built with
Vite, and the Rust side in `apps/shell/src-tauri/`, a Cargo workspace of its own. Beside what
the [README](https://github.com/lein3000zzz/neon-teamspace#development) lists, cargo-about
among them, a build needs [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/)
for its platform:

- **Linux**: WebKitGTK 4.1 and the other libraries Tauri lists, on Debian or Ubuntu
  `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev
  libayatana-appindicator3-dev librsvg2-dev`.
- **macOS**: Xcode's command-line tools for the desktop app; full Xcode and CocoaPods
  (`brew install cocoapods`) for iOS.
- **Windows**: the Microsoft C++ build tools and WebView2.
- **Android**: JDK 17, the Android SDK with its platform tools and NDK, and the Rust targets
  `aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`.
  `JAVA_HOME`, `ANDROID_HOME` and `NDK_HOME` must point at them.

The Makefile runs everything:

```sh
make app-install    # the picker's packages and Tauri's CLI, pinned in apps/shell/bun.lock
make app-dev        # the desktop app, the picker reloading as it changes
make app-check      # the picker's types, lints and formatting; the Rust side's formatting,
                    # clippy and tests (part of make check)
make app-browsers   # once: the headless Chromium and WebKit for the next line
make app-test       # the picker on its own, in Chromium and WebKit at desktop and phone sizes
make app-build      # this machine's desktop app and installers
make app-webview-check  # on Linux, after make app-build: the app against a hostile server
make app-android    # an APK for every ABI, signed with the debug key, for testing
make app-ios        # the iOS app for TestFlight
```

Each build of the picker writes `THIRD_PARTY_NOTICES.txt` beside it, with the license
notices of the picker's packages and, through cargo-about, of the crates the apps link, and
for each one under MPL-2.0 its address on npm or crates.io and its repository, where its
source is. The app embeds it with the picker, whose **Third-party licenses** link opens it.

`make app-build` writes to `apps/shell/src-tauri/target/release/bundle/`: on macOS the `.app`
and a `.dmg`, on Windows the `.msi` and setup `.exe`, on Linux the `.deb`, `.rpm` and AppImage.
It sets `CI=true`, which makes the disk image's script skip the AppleScript that arranges its
Finder window; that script needs permission to control Finder. `make app-android` writes
`apps/shell/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`,
which `adb install` puts on a device or an emulator.

`make app-webview-check` runs the built Linux app in WebKitGTK against a stand-in server whose
page calls the picker's commands while it sends the window to the app's own origin, and fails
if one call goes through. It needs a display: CI runs it under `xvfb-run -a`.

### iOS

The iOS app builds on a Mac with full Xcode, and is signed with an Apple developer team's
certificate: set `APPLE_DEVELOPMENT_TEAM` to the team's ID, or `bundle.iOS.developmentTeam` in
`apps/shell/src-tauri/tauri.conf.json`. `make app-ios` archives the app and exports it for App
Store Connect, from where Xcode's Organizer or Apple's Transporter uploads it to TestFlight.
`bun run tauri ios dev` in `apps/shell` runs it in a simulator. The Xcode project is generated
in `apps/shell/src-tauri/gen/apple/`; `bun run tauri ios init` there makes it again after
Tauri's template changes.

### Releases

The release workflow builds the desktop apps on GitHub's Linux (Ubuntu 22.04), Windows and
macOS runners, and the Android APK on Linux, attests their build provenance, and attaches them
to the release with `SHA256SUMS`. Like the server's images, they build from the tag's checkout
alone: nothing comes from the Actions cache, neither crates built before nor Bun, so the
provenance vouches only for what the tag holds, and every release builds them from scratch.
Each job that builds or signs an app passes the SHA-256 of its files on to the next as a job
output, and the job that signs or attaches a file stops when it does not match, as another job
of the run could replace an artifact. The job that attaches the files attests them too, once
it has checked them and every other job passed: the jobs that build them run the crates' build
scripts and the packages' code, and get no OIDC token, with which that code could attest any
file as the workflow's. It builds the APK unsigned, and once the end-to-end tests passed signs
it in a job of its own, which runs only the Android SDK's `zipalign` and `apksigner`, when the
repository has these secrets:

| Secret | What it holds |
|---|---|
| `ANDROID_KEY_BASE64` | The release keystore, as `base64 -i release.jks` prints it. |
| `ANDROID_KEY_ALIAS` | The alias of the key in it. |
| `ANDROID_KEY_PASSWORD` | The password of the keystore and of the key. |

Make the keystore with `keytool -genkey -v -keystore release.jks -keyalg RSA -keysize 2048
-validity 10000 -alias upload`, and keep it out of the repository. Only the signing job gets
the secrets, and it writes the keystore outside any checkout and deletes it once used, so no
build script, Gradle plugin or package can read the key. Without the secrets, the APK is
attached unsigned, with `-unsigned` in its name.
