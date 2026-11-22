// Which rate limits are worth waiting out and retrying. Run: npm test
import assert from 'node:assert/strict';
import { rateWait } from '../limits.mjs';

const T = 1_000_000;
// Gemini's per-minute limit names its own delay.
assert.equal(rateWait(new Error('429 You exceeded your current quota, please check your plan and billing details. Please retry in 37.5s.'), T), 39_500);
assert.equal(rateWait(new Error('Rate limit reached: tokens per minute (TPM): Limit 7000, Requested 14362. Please try again in 20s'), T), 22_000);
// No delay given: a minute.
assert.equal(rateWait(new Error('429 Too Many Requests'), T), 60_000);
assert.equal(rateWait({ message: 'rate limit exceeded', status: 429 }, T), 60_000);
// A retry-after header counts, with a floor of ten seconds.
assert.equal(rateWait({ message: 'rate limited', headers: { 'retry-after': '1' } }, T), 12_000);
assert.equal(rateWait({ message: 'rate limited', headers: { 'retry-after': '45' } }, T), 47_000);
// Long waits, daily or monthly quotas, an empty account and other errors aren't waited on.
assert.equal(rateWait(new Error('rate limit hit. Please retry in 20 minutes'), T), null);
assert.equal(rateWait(new Error('Quota exceeded for metric: requests per day. Please retry in 30s'), T), null);
assert.equal(rateWait(new Error('You exceeded your daily quota'), T), null);
assert.equal(rateWait(new Error('insufficient_quota: check your plan and billing details'), T), null);
assert.equal(rateWait(new Error('That key is invalid'), T), null);
assert.equal(rateWait(new Error('fetch failed'), T), null);
assert.equal(rateWait(null, T), null);
console.log('ratewait ok');
