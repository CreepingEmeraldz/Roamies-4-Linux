// The Commands page's rules. Run: npm test
import assert from 'node:assert/strict';
import { DEFAULTS, cleanSaved, decide, findSaved, instantMatch, matches, normalize, savedLine } from '../commands.mjs';

// Patterns: plain words match that command with any arguments; * matches anything.
assert.ok(matches('git status', 'git status'));
assert.ok(matches('git status', 'git status -s'));
assert.ok(!matches('git status', 'git statusx'));
assert.ok(!matches('git', 'gitk'));
assert.ok(matches('niri msg *', 'niri msg action focus-column-right'));
assert.ok(matches('cat *.log', 'cat app.log') && !matches('cat *.log', 'ls app.log'));
assert.ok(!matches('', 'ls') && !matches('ls', ''));
assert.ok(matches('rm (x)*', 'rm (x)y'), 'regex characters are plain text');

// Normalising: bad input becomes the defaults, lists are trimmed and unique.
assert.deepEqual(normalize(null), DEFAULTS);
assert.equal(normalize({ mode: 'whatever' }).mode, 'smart');
assert.deepEqual(normalize({ blocked: [' sudo ', 'sudo', '', 5] }).blocked, ['sudo']);
const s = cleanSaved({ name: ' Next window ', phrases: 'next window, move to next window', command: 'niri msg action focus-column-right', ask: 'never' });
assert.deepEqual(s, { id: 'next-window', name: 'Next window', phrases: ['next window', 'move to next window'], command: 'niri msg action focus-column-right', ask: 'never', instant: true });
assert.equal(cleanSaved({ name: 'x' }), null);
assert.equal(normalize({ saved: [s, { ...s }] }).saved.length, 1, 'one per name');

// Smart (the default): only risky commands ask.
assert.equal(decide('ls', 'safe').action, 'run');
assert.equal(decide('mv a b', 'ask').action, 'ask');
// Always ask, never ask.
assert.equal(decide('ls', 'safe', { mode: 'always' }).action, 'ask');
assert.equal(decide('mv a b', 'ask', { mode: 'never' }).action, 'run');
// A list: no asking for what's on it (all parts of it), asking for the rest.
const listed = { mode: 'listed', trusted: ['git status', 'niri msg *'] };
assert.equal(decide('git status -s', 'safe', listed).action, 'run');
assert.equal(decide('git status && rm -rf x', 'ask', listed).action, 'ask', 'one unlisted part asks');
assert.equal(decide('niri msg action close-window', 'safe', listed).action, 'run');
assert.equal(decide('ls', 'safe', listed).action, 'ask');
// A blocked kind is blocked in every mode, even chained, and even for a saved command that never asks.
const blocked = { mode: 'never', blocked: ['sudo', 'rm -rf *'] };
assert.equal(decide('sudo apt update', 'ask', blocked).action, 'block');
assert.equal(decide('echo hi && sudo reboot', 'ask', blocked).action, 'block');
assert.equal(decide('rm -rf ~/x', 'ask', blocked).action, 'block');
assert.equal(decide('sudo ls', 'safe', blocked, 'never').action, 'block');
assert.equal(decide('ls', 'safe', blocked).action, 'run');
// A saved command's own choice beats the mode.
assert.equal(decide('ls', 'safe', { mode: 'always' }, 'never').action, 'run');
assert.equal(decide('ls', 'safe', { mode: 'never' }, 'always').action, 'ask');

// Finding a saved command by name, id or phrase.
assert.equal(findSaved([s], 'next window').id, 'next-window');
assert.equal(findSaved([s], 'Move to next window').id, 'next-window');
assert.equal(findSaved([s], 'nope'), null);
assert.match(savedLine([s]), /Next window \("next window", "move to next window"\)/);
assert.equal(savedLine([]), '');
// Instant commands: off by default, and only when the whole message is the name or a phrase.
const inst = { instant: true, saved: [s, { ...s, id: 'quiet', name: 'Quiet one', phrases: ['shh'], instant: false }] };
assert.equal(normalize({}).instant, false);
assert.equal(instantMatch({ saved: [s] }, 'next window'), null, 'off by default');
assert.equal(instantMatch(inst, 'next window').id, 'next-window');
assert.equal(instantMatch(inst, '  Next Window.  ').id, 'next-window', 'case and punctuation do not matter');
assert.equal(instantMatch(inst, 'move to next window').id, 'next-window', 'a phrase');
assert.equal(instantMatch(inst, 'my friend said put it next window not this window'), null, 'a sentence is not the phrase');
assert.equal(instantMatch(inst, 'next window please'), null, 'nothing extra');
assert.equal(instantMatch(inst, 'shh'), null, 'a command switched off for instant');
assert.equal(instantMatch(inst, ''), null);
assert.equal(cleanSaved({ name: 'x', command: 'y', instant: false }).instant, false);
console.log('commands ok');
