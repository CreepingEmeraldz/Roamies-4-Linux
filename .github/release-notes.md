## Roamies knows which window you're in

Before each question, Roamies now checks which window has your cursor, over the accessibility bus, and tells the AI. So if you're on a Wikipedia page in Brave, you no longer have to say so: it knows you're in Brave, on that page, and won't try to take you there again. This is what the macOS and Windows builds already did.

It reads the window's title (a browser puts the page name in it), so a page with a generic title gives it less to go on than one with a clear one.

## Clearer update messages

When the update check fails, it now says why. If GitHub is limiting checks from your connection, you get "GitHub is limiting update checks. Try again soon." If it's down, "GitHub isn't answering. Try again later." Before, both showed "The update didn't work (E40)". The update button still opens the releases page, since Linux builds don't install themselves yet.
