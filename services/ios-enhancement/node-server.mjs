import { createServer } from 'node:http';
import { Readable } from 'node:stream';
import { createHandler } from './worker.mjs';
import { BudgetLedger } from './budget.mjs';
import { FileStorage } from './file-storage.mjs';

// One Node process only. TLS terminates at Nginx; this listener is loopback-only.
const ledger = new BudgetLedger(new FileStorage(process.env.BUDGET_FILE ?? './private-state/budget.json'));
const handler = createHandler({ limiter: (id, characters) => ledger.reserve(id, characters) });
const server = createServer({ maxHeaderSize: 8192 }, async (incoming, outgoing) => {
  const abort = new AbortController();
  incoming.on('aborted', () => abort.abort());
  outgoing.on('close', () => { if (!outgoing.writableEnded) abort.abort(); });
  try {
    const method = incoming.method ?? 'GET';
    const request = new Request(new URL(incoming.url ?? '/', 'http://localhost'), { method, headers: incoming.headers, signal: abort.signal, ...(method !== 'GET' && method !== 'HEAD' ? { body: Readable.toWeb(incoming), duplex: 'half' } : {}) });
    const response = await handler(request, process.env);
    outgoing.writeHead(response.status, Object.fromEntries(response.headers));
    if (response.body) Readable.fromWeb(response.body).pipe(outgoing);
    else outgoing.end();
  } catch {
    if (!outgoing.headersSent) outgoing.writeHead(503, { 'cache-control': 'no-store' });
    outgoing.end();
  }
});
server.requestTimeout = 5000;
server.headersTimeout = 5000;
server.keepAliveTimeout = 1000;
server.maxConnections = 32;
server.listen(Number(process.env.PORT ?? 3107), '127.0.0.1');
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.close(() => process.exit(0)));
