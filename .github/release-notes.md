## Based on Roamies 0.1.50

This release brings in everything from upstream Roamies 0.1.50: fixes for the nine CodeQL security alerts, the Model page no longer showing a job's model as the default, and fixes for broken app steps and missing-file opens.

## A Commands page

There's a new **Commands** page in the panel for how Roamie runs shell commands.

- **When to ask.** Choose between asking when a command looks risky (as before), asking every time, never asking, or never asking only for the commands on your own list (and asking for everything else). "Never ask" and your list skip only the yes: anything the built-in safety rules block stays blocked.
- **Never run.** A list of kinds of command Roamie must refuse in every mode, such as `sudo` or `rm -rf *`. It also catches them when chained (`echo hi && sudo reboot`).
- **My commands.** Save a command with a name and the phrases you'd say, for example "next window" for `niri msg action focus-column-right`. Roamie runs it by name, so the AI can't change what runs, and each one can be set to always ask, never ask, or follow the general setting.

In a list, `git status` also covers `git status -s`, and `*` stands for anything (`niri msg *`).
