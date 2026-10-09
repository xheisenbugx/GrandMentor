# Use GrandMentor on your phone

GrandMentor runs on your computer. With **phone access** turned on, any phone or tablet on the same
Wi-Fi can open it through a secure (`https://`) address, enter a PIN once, and install it as an app
(it then opens full screen from the home screen, like any other app).

Nothing changes until you turn it on: by default GrandMentor only listens on `127.0.0.1` and is
invisible to other devices.

---

## Quick start

1. On the computer, open **Settings → Use on your phone** and switch on **Phone access**
   (or start the server with `GM_LAN=1`).
2. Restart GrandMentor. The section now shows two QR codes and a 6-digit PIN.
3. On the phone (same Wi-Fi):
   1. **Install the security certificate** — scan the first QR code (or open the link) to
      download `grandmentor-ca.crt`, then follow the steps for your phone below. Only once.
   2. **Open GrandMentor** — scan the second QR code (`https://<computer-ip>:8443/`).
   3. **Enter the PIN** shown on the computer. The phone stays signed in.
   4. **Install the app** — Chrome menu (⋮) → **Install app** (or **Add to Home screen**).

### Android (Chrome)

Chrome on Android trusts certificate authorities that you install yourself, so after this the
address shows a normal padlock and the app can be installed.

1. Scan the certificate QR code with the camera (or open the link in Chrome) to download
   `grandmentor-ca.crt`.
2. Open **Settings → Security** (on some phones **Security & privacy**) → **More security settings**
   → **Encryption & credentials** → **Install a certificate** → **CA certificate**.
3. Tap **Install anyway** (Android warns about every CA certificate; this one is limited to your
   home network, see *Security notes*). Confirm with your screen lock if asked.
4. Pick `grandmentor-ca.crt` from **Downloads**.
5. Open the app address in Chrome, enter the PIN, then menu (⋮) → **Install app**.

To remove it later: **Encryption & credentials → User credentials** (or **Trusted credentials →
User**) → GrandMentor local CA → **Remove**.

### iPhone / iPad (Safari)

1. Open the certificate link in **Safari** and tap **Allow** to download the profile.
2. **Settings → General → VPN & Device Management** → GrandMentor local CA → **Install**.
3. **Settings → General → About → Certificate Trust Settings** → switch on
   **GrandMentor local CA** (iOS does not trust it for websites until you do this).
4. Open the app address in Safari, enter the PIN, then **Share → Add to Home Screen**.

---

## How it works

| | Address | Who can use it |
|---|---|---|
| Plain HTTP | `http://localhost:8080` (`GM_PORT`) | This computer (never asks for a PIN) |
| HTTPS | `https://<lan-ip>:8443` (`GM_LAN_PORT`) | Phones and tablets after the PIN; this computer too |

* **Certificates.** On first start in phone mode GrandMentor creates a small certificate authority
  (`ca.crt.pem`, valid 10 years) and a server certificate signed by it for this computer's LAN
  addresses, `<hostname>`, `<hostname>.local` and `localhost` (valid 397 days, renewed
  automatically 30 days before it expires). Every two minutes it checks the network addresses; when
  the computer gets a new address (another Wi-Fi, a new DHCP lease) the server certificate is
  re-issued and swapped in without a restart. The CA stays the same, so phones never need to
  re-install it.
* **Plain HTTP in phone mode.** The HTTP port also listens on the network (unless you set
  `GM_HOST` yourself) but, for other devices, it only serves the certificate download
  (`/phone/ca.crt`) and device sync; everything else is redirected to the HTTPS address. This lets
  a phone download the certificate before it trusts HTTPS.
* **PIN.** Every request that does not come from this computer (pages, API, live-analysis
  websocket, static files) needs a sign-in cookie, except `/login`, the app manifest and its
  icons. The phone types the PIN once on the small `/login` page and gets a cookie valid for one
  year (`HttpOnly`, `SameSite=Strict`, and `Secure` on HTTPS). The computer itself never needs the
  PIN. The PIN can be replaced (**New PIN**) and all phones can be signed out (**Sign out all
  devices**) from Settings; a phone can sign itself out from its own Settings page.
* **Wrong PINs** are limited to 5 per device per 15 minutes and 30 in total per hour (HTTP 429 with
  `Retry-After`). PINs and cookies are compared in constant time.
* **Files** live in `GM_PHONE_DIR` (default: a `grandmentor-phone/` folder next to the database):
  `phone.json` (the switch), `access.json` (PIN + signed-in devices, owner-only),
  `ca.key.pem` / `server.key.pem` (owner-only), `ca.crt.pem`, `server.crt.pem` and small `.json`
  notes. Deleting the folder resets everything: new CA (phones must re-install it), new PIN,
  everyone signed out.

## Settings and environment variables

| Variable | Default | Meaning |
|---|---|---|
| `GM_LAN` | unset | `1` forces phone mode on, `0` forces it off (overrides the Settings switch, which is then read-only). |
| `GM_LAN_PORT` | `8443` | HTTPS port for phones. |
| `GM_PHONE_DIR` | `<folder of GM_DB>/grandmentor-phone` | Certificates, PIN, signed-in devices, `phone.json`. |
| `GM_ACCESS_PIN` | on | `off` lets other devices in **without** a PIN. Only for networks where every device is trusted (e.g. a private VPN). Not recommended. |
| `GM_HOST` | `127.0.0.1` | Address of the plain HTTP port. `0.0.0.0` without phone mode serves the whole app over plain HTTP to the network — now protected by the PIN, but without HTTPS phones can't install the app and the cookie travels unencrypted. |

## Device sync between two computers

The "Connect to another device" sync (Settings → Your data) keeps working with phone mode and the
PIN: `/api/sync/snapshot` and `/api/sync/merge` carry their own short-lived pairing code
(`X-GM-Pair`) and are therefore not asked for the PIN, on HTTP or HTTPS. Use the computer's plain
address (`http://<ip>:8080`) in the sync form; in phone mode the HTTP port is reachable from the
network for exactly this purpose. Creating a pairing code is still only possible on the computer
itself, and `GM_SYNC_ALLOW_ORIGINS` still restricts which origins may call those two endpoints.

## Troubleshooting

* **The phone can't open the address.** Both devices must be on the same Wi-Fi (guest networks
  often block devices from seeing each other). Check that the computer's firewall allows
  GrandMentor (macOS asks the first time; on Windows allow it for *Private networks*). If the
  computer has several addresses, pick another one in Settings.
* **"Your connection is not private".** The certificate isn't installed (or, on iPhone, full trust
  isn't switched on). Install it as above. If you reset `GM_PHONE_DIR`, remove the old certificate
  from the phone and install the new one.
* **The certificate link downloads nothing on Android.** Open it in Chrome (not an in-app browser),
  then install it from Settings as described; Android no longer installs CA certificates straight
  from the browser.
* **Asked for the PIN again.** Someone used **Sign out all devices**, the phone's cookies were
  cleared, or the phone was signed in more than a year ago.
* **"Too many tries".** Wait for the time shown (at most 15 minutes for one device, one hour if many
  wrong PINs came in), then check the PIN on the computer.
* **`<hostname>.local` doesn't work.** Not every phone resolves `.local` names; use the IP address
  (the QR codes always do).
* **Port in use.** Set `GM_LAN_PORT` (and `GM_PORT`) to free ports.

## Security notes

* The CA's private key never leaves the computer (`ca.key.pem`, readable only by your user). The CA
  is **name-constrained**: it can only vouch for private addresses (`10/8`, `172.16/12`,
  `192.168/16`, `100.64/10`, `169.254/16`, `127/8`), `*.local`, `localhost` and this computer's
  host name. Even if someone stole the key, a phone that trusts it would still reject certificates
  for real websites.
* Anyone on your network who knows the PIN can use GrandMentor and see your games. Use **New PIN**
  and **Sign out all devices** if you shared it with someone you no longer want to have access.
* Requests through a reverse proxy running on the same computer look like they come from the
  computer itself and bypass the PIN; put your own authentication in front of such a proxy.
* Turn phone access off (and restart) when you don't need it.
