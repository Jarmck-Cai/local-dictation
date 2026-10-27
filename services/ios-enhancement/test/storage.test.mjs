import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { FileStorage } from '../file-storage.mjs';
import { BudgetLedger } from '../budget.mjs';

test('VPS budget persists across process restart and fails closed on corruption', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'dictation-budget-'));
  const filename = join(directory, 'budget.json');
  try {
    const first = new BudgetLedger(new FileStorage(filename));
    assert.equal((await first.reserve('same', 100, 1000)).allowed, true);
    const restarted = new BudgetLedger(new FileStorage(filename));
    assert.equal((await restarted.reserve('same', 100, 1100)).allowed, false);
    await writeFile(filename, 'corrupted');
    await assert.rejects(restarted.reserve('new', 100, 1200));
  } finally { await rm(directory, { recursive: true, force: true }); }
});
test('VPS adapter serializes overlapping transactions', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'dictation-budget-'));
  try {
    const ledger = new BudgetLedger(new FileStorage(join(directory, 'budget.json')));
    const result = await Promise.all(Array.from({ length: 12 }, (_, i) => ledger.reserve(`id${i}`, 100, 1000)));
    assert.equal(result.filter(x => x.allowed).length, 6);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
