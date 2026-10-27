export class BudgetLedger {
  constructor(storage) { this.storage = storage; }
  async reserve(requestId, characters, now = Date.now()) {
    if (typeof requestId !== 'string' || !Number.isInteger(characters) || characters < 1 || characters > 8000) return { allowed: false };
    return this.storage.transaction(async transaction => {
      const recent = (await transaction.get('reservations') ?? []).filter(x => x.time > now - 86_400_000);
      if (recent.some(x => x.id === requestId) || recent.filter(x => x.time > now - 60_000).length >= 6 || recent.length >= 100 || recent.reduce((sum, x) => sum + x.characters, 0) + characters > 100_000) return { allowed: false };
      recent.push({ id: requestId, time: now, characters });
      await transaction.put('reservations', recent);
      return { allowed: true };
    });
  }
}

// SQLite-backed Durable Object; only request identifiers, timestamps and lengths are stored.
// No transcript, credential or provider reply is persisted.
export class PrototypeBudget {
  constructor(state) { this.ledger = new BudgetLedger(state.storage); }
  async fetch(request) {
    const { requestId, characters } = await request.json();
    return Response.json(await this.ledger.reserve(requestId, characters));
  }
}
