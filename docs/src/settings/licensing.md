# Licensing

dirmap is being released as **Filescape**. The file browser, the disk map and basic filtering are
free for good. The rest is **Filescape Professional**, unlocked by a licence. Open
**⚙ Settings > Licensing**, or **Help > Licence**, to see your licence or enter one.

## Free and Professional features

| Free | Filescape Professional |
|---|---|
| The Files workspace: browsing, tabs, the address bar, the preview, opening files, copy, move, rename, delete and New | The **Custom** filter, filter presets and recent filters |
| Scanning, the treemap, the sunburst, the directory tree and the File types list | Finding numbered sequences, and the sequence player |
| The **Show:** category buttons, size limits, Highlight and Show matches only | The Remote workspace: connecting to servers and starting transfers |
| Scan exclusions | Batch rename, Compare Folders and the duplicate finder |
| | Browsing and extracting archives, saved searches and snapshots |

Professional features are checked only when you **start** something. Work that's already running,
such as a copy, a transfer, a batch rename or an extraction, always finishes, even if a licence
expires halfway through. Retrying or resuming a transfer is never checked. When a feature needs a
licence, a box says so and offers **Enter Licence Key…**, **Purchase Perpetual Licence** and
**Not now**. Your files, settings and saved data are never touched.

## Evaluation

For the first 15 days after Filescape is first started on a computer, every feature is unlocked.
In the last week a line at the top of the window counts the days down. Afterwards the free features carry on
as before. The start date is kept in the system keychain as well as in Filescape's own files, so
a temporary licence ending never starts the evaluation again.

## Licence types

| Type | What it does |
|---|---|
| **Perpetual** | Never expires. It covers every version released while its updates last (12 months from purchase), and can be used on up to 10 computers. |
| **Temporary** | Full access for a set time: usually 30 days from the first time it's activated, or until a fixed date. It can't be renewed; when it ends you can enter another licence or buy a perpetual one. |

If you have both, the perpetual licence is used. When a temporary licence expires, Filescape
shows **Your temporary Filescape licence has expired**, Professional features ask for a licence
again, and everything you'd saved is kept.

## Entering a licence

Type or paste the serial number (it starts with `FST-` for a temporary licence or `FSP-` for a
perpetual one) into the box and press **Activate**. Spaces, dashes and the case of letters don't
matter, and a mistyped serial is caught at once without using an activation.

The status screen then shows the licence type, the expiry date (or the end of updates for a
perpetual licence), the days remaining and how many activations have been used. **Licence
Details** lists the registered owner, licence ID, serial number, and the dates issued, activated
and expiring.

| Button | Effect |
|---|---|
| **Copy Serial** | Copies the serial number to the clipboard. |
| **Export Licence ⏷ > Licence Certificate (signed)…** | Saves a `.filescape-licence` file signed by Filescape, as proof of purchase. **Verify a licence certificate** checks one. |
| **Export Licence ⏷ > Licence Details (text)…** | Saves the details as plain text. |
| **Deactivate This Computer…** | Ends the licence on this computer. The activation it used isn't given back. |
| **Purchase Perpetual Licence** | Opens the Filescape website. |

## Activations

Each new computer a licence is activated on uses one **activation**. A temporary licence has 3, a
perpetual one 10.

- Starting Filescape again, restarting the computer or activating again on the same computer
  doesn't use another one.
- Deactivating a computer doesn't give its activation back. For a temporary licence with 3
  activations: activate on computer A (1 used), deactivate A, activate on B (2 used), then on C (3
  used). A fourth computer is refused, even if A, B and C are all deactivated.
- A temporary licence's expiry date is set by its first activation and never moves.

If you replace computers and run out, ask Filescape support: they can reset a licence's activations.

## Offline activation

For a computer without internet access, open **Offline activation**, enter the serial and choose
**Save Request…**. Take the request file to any computer that's online, upload it at
`account.filescape.app/activate`, and bring the response file back with **Import Response…**. It
counts towards the same limit as activating online, and the response only works on the computer
that made the request.

## Checks and the clock

Licences are signed by Filescape's licence server, so changing the files Filescape keeps doesn't
unlock anything. Once a week, when the computer is online, Filescape checks that the licence is
still valid. A failed check (no internet, server down) never locks anything.

Time-limited licences go by the computer's clock. If the clock is set back more than two hours
before a time Filescape has already seen, time-limited licences pause and the banner says
**This computer's clock is behind**. Set the clock right, or go online so Filescape can check the
time. Perpetual licences don't depend on the clock.

A computer kept offline for good can't be checked, so a determined person could keep a temporary
licence going there by changing the clock and Filescape's files together. The weekly check catches
this as soon as the computer goes online.

## Test mode

Builds made for testing can run against a licence server built into the app. Start Filescape with
the environment variable `FILESCAPE_LICENCE_DEV=1` and the Licensing tab gets a **Test mode**
section: issue temporary or perpetual serials, then use them as a customer would. **Licence
clock** moves the licence's time forward so you can see a licence expire without waiting. Test
serials work only on the computer that issued them, and release builds don't have test mode.

## Related topics

- [Settings reference](reference.md)
- [Feature status](../about/feature-status.md)
- [Frequently asked questions](../troubleshooting/faq.md)
