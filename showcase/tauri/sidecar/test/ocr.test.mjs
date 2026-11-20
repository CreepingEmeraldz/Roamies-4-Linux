import test from 'node:test';
import assert from 'node:assert/strict';
import { parseTsv, findWords, pngSize } from '../ocr.mjs';

const head = 'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext';
const row = (b, l, left, top, w, h, text, conf = 90) => `5\t1\t${b}\t1\t${l}\t1\t${left}\t${top}\t${w}\t${h}\t${conf}\t${text}`;
const tsv = [head, row(1, 1, 100, 20, 60, 16, 'Search'), row(1, 1, 170, 20, 80, 16, 'Wikipedia'), row(2, 1, 900, 20, 70, 16, 'Search'), row(3, 1, 10, 200, 40, 14, 'Log'), row(3, 1, 60, 200, 20, 14, 'in'), row(4, 1, 5, 5, 9, 9, 'noise', 5)].join('\n');

test('words come out of the TSV, low confidence dropped', () => {
  const w = parseTsv(tsv);
  assert.equal(w.length, 5);
  assert.deepEqual([w[0].x, w[0].y, w[0].w, w[0].h], [100, 20, 60, 16]);
});
test('a word is found everywhere it is, in reading order', () => {
  const hits = findWords(parseTsv(tsv), 'search');
  assert.deepEqual(hits.map((h) => h.bounds[0]), [100, 900]);
});
test('a phrase must be consecutive words on one line, and its box joins them', () => {
  const w = parseTsv(tsv);
  assert.deepEqual(findWords(w, 'Search Wikipedia')[0].bounds, [100, 20, 150, 16]);
  assert.deepEqual(findWords(w, 'Log in')[0].bounds, [10, 200, 70, 14]);
  assert.equal(findWords(w, 'Wikipedia Search').length, 0);
  assert.equal(findWords(w, 'Search Log').length, 0);
});
test('punctuation and case do not matter, a single letter must match exactly', () => {
  const w = parseTsv([head, row(1, 1, 1, 1, 5, 5, 'Log-in:'), row(1, 1, 20, 1, 5, 5, 'x'), row(1, 1, 30, 1, 5, 5, 'X')].join('\n'));
  assert.equal(findWords(w, 'log in').length, 0, 'one OCR word is not two query words');
  assert.equal(findWords(w, 'login').length, 1);
  assert.deepEqual(findWords(w, 'X').map((h) => h.bounds[0]), [30]);
});
test('a PNG header gives its size', () => {
  const b = Buffer.alloc(32); b.write('\x89PNG', 0, 'latin1'); b.writeUInt32BE(2256, 16); b.writeUInt32BE(1504, 20);
  assert.deepEqual(pngSize(b), [2256, 1504]);
  assert.equal(pngSize(Buffer.alloc(8)), null);
});
