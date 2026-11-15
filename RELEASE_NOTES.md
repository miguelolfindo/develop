## Develop 0.2.1

### Browse your network
The folder tree now has a **Network** entry at the bottom, for photos kept on a NAS or
another computer.

- **Computers and NAS boxes** on your network are listed under Network:
  - **Windows:** the servers behind your mapped drives, computers you added or opened
    photos from, and whatever Windows network discovery reports.
  - **macOS:** computers that advertise file sharing (Bonjour), and shares you already
    have mounted.
- Open a computer to see its **shared folders**, then the folders inside them, just like
  a local drive.
- **Add a computer or share by address** with the field under Network:
  `\\computer` or `\\computer\share` on Windows, `computer` or `smb://computer/share` on
  macOS (an IP address works too). Develop remembers it. Right-click a computer for
  **Read again** or **Remove from Network**.
- **Signing in:**
  - **Windows** shows its own sign-in dialog when a computer needs a password.
  - **macOS:** clicking a share asks Finder to mount it (using your Keychain, or asking
    for a password). Once it is mounted, Develop opens it. If a computer will not list its
    shares without signing in, use **Connect…**.
- **Mapped drives** now show which share they are, for example `W:  \\server\photos`.
- Network lookups run in the background, so a slow or sleeping NAS never freezes the
  window.
- NAS system folders such as `#recycle`, `#snapshot` and `@eaDir` are hidden from the tree.

### Upgrading from 0.1.x?
Version 0.2.0 added Nikon, Canon, Panasonic and Fujifilm raw files (including X-Trans),
sliders that match Lightroom much more closely, and a built-in lens profile database. See
the [0.2.0 release notes](https://github.com/miguelolfindo/develop/releases/tag/v0.2.0).

### Installing
- **Windows:** run `develop_0.2.1_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** open `Develop_0.2.1_aarch64.dmg` and
  drag Develop to Applications. The app is not notarized: the first time, right-click
  Develop in Applications, choose **Open**, then **Open** again.

### Known limitations
- Windows network discovery only finds computers that announce themselves the old way;
  many NAS boxes do not. Add those once by address, or map a drive to them.
- On macOS, finding computers relies on Bonjour; computers that do not advertise file
  sharing need adding by address.
- Dehaze remains the slider furthest from Lightroom.
