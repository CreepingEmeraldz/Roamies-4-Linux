## The rate-limit notice counts down and lets you carry on

The "rate limiting me" notice now counts down ("Trying again in 45s", then 44s, and so on) instead of showing the full wait. It also no longer takes over your keyboard, so you can keep typing in other windows while it waits. The Cancel button still works.

## A fix for stray pixels left on screen

Stray bits of a closed bubble could stay on screen until something redrew that spot, and they went away while Roamie moved and came back when it stopped. After any change, Roamie now keeps redrawing for a few more frames so every screen buffer gets the new picture, then redraws once a second as a safety net.
