import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BudgetLedger } from '../budget.mjs';

class Storage {
  constructor() { this.values = new Map(); this.serial = Promise.resolve(); }
  async get(k) { return structuredClone(this.values.get(k)); }
  async put(k, v) { this.values.set(k, structuredClone(v)); }
  async transaction(body) {
    const job = this.serial.then(() => body(this));
    this.serial = job.catch(() => {});
    return job;
  }
}
test('reservation survives instance replacement and rejects replays', async () => {
  const storage = new Storage();
  assert.equal((await new BudgetLedger(storage).reserve('request-1', 100, 1000)).allowed, true);
  assert.equal((await new BudgetLedger(storage).reserve('request-1', 100, 1100)).allowed, false);
  assert.ok(!JSON.stringify([...storage.values]).includes('text'));
});
test('parallel reservations cannot exceed six per minute', async () => {
  const storage = new Storage();
  const replies = await Promise.all(Array.from({ length: 20 }, (_, i) => new BudgetLedger(storage).reserve(`r${i}`, 100, 1000)));
  assert.equal(replies.filter(r => r.allowed).length, 6);
});
test('rolling day limits remain enforced after minute elapses', async () => {
  const storage = new Storage();
  const ledger = new BudgetLedger(storage);
  for (let i = 0; i < 100; i++) assert.equal((await ledger.reserve(`r${i}`, 50, i * 61000)).allowed, true);
  assert.equal((await ledger.reserve('last', 50, 100 * 61000)).allowed, false);
  assert.equal((await ledger.reserve('next-day', 50, 86_400_001)).allowed, true);
});
test('daily character budget prevents large-text cost amplification', async () => {
  const ledger = new BudgetLedger(new Storage());
  for (let i = 0; i < 12; i++) assert.equal((await ledger.reserve(`r${i}`, 8000, i * 61000)).allowed, true);
  assert.equal((await ledger.reserve('overflow', 8000, 13 * 61000)).allowed, false);
  assert.equal((await ledger.reserve('remainder', 4000, 14 * 61000)).allowed, true);
});
