## Roamies waits out a rate limit instead of giving up

When a provider says you're going too fast (Gemini's free tier does this a lot), and no backup model is set up to take over, Roamies now says so, for example "Gemini is rate limiting me. I'll try again in a minute", with a **Cancel** button. When the time is up, it carries on from where it stopped. It uses the wait the provider asks for when it gives one, and a minute when it doesn't.

It tries up to three times. It doesn't wait for daily or monthly quotas or an empty account, since those won't clear in minutes, and shows the usual message instead. Sending a new message also ends the wait.

## The text box keeps clear of Roamie

Instead of only choosing a side, the type-to-ask box now looks at where Roamie actually is and takes whichever corner of the cursor (below or above, left or right) doesn't overlap him, including at the screen edges.

## Roamie can scroll other apps

In another app, Roamie can now scroll the page or list in front by pages, or jump to the end of a feed that loads more as you go (up and down, left and right). It presses Page Up and Page Down and Home and End through `wtype`, so it needs `wtype` installed, the same as typing. If a text box has the cursor, it asks to click out of it first, since the keys would go into the box.
