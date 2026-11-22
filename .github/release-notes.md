## The AppImage now updates itself

When a newer release is out, Roamies tells you, and **Install and Restart** in its tray menu downloads it, checks its signature, swaps it in for the old AppImage and restarts. You no longer have to download the file yourself. Your settings, keys and memory are kept.

Releases are now signed with this fork's own key, and each one carries a `latest.json` file that the updater reads. The `.deb` and `.rpm` copies can't replace themselves, so they still open the download page.

**One manual step this time:** earlier builds don't have this key, so they can't update to this one. Download this AppImage and run it in place of your old one. From the next release on, the update button does it for you.
