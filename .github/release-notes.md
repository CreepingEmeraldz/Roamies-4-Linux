## The model menu shows only what your key can run

Before, the Model menu listed every model a provider has, and picking one your key couldn't use failed when you asked something. Now Roamies asks each provider which models your key can actually use and leaves the rest out of the menu.

- **OpenRouter:** the list now follows your account's own settings (privacy, provider preferences and guardrails), so models you've ruled out are gone.
- **OpenAI, Anthropic, Gemini, xAI, DeepSeek and Kimi:** with an API key saved, models the key can't call are hidden.
- If a provider doesn't answer, or its list looks odd, nothing is hidden and the menu is as it was. If you're signed in with Claude or ChatGPT instead of a key, nothing changes either.
- If the model you had picked turns out to be one your key can't run, Roamies switches to one it can.

The list is checked when you save a key and again every few hours.

## The text box no longer sits on top of Roamie

When you open the type-to-ask box, it now goes on the opposite side of the cursor from Roamie, so the two don't overlap. If there's no room on that side, it falls back to the old spot.
