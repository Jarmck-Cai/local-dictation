import { mkdir, open, readFile, rename } from 'node:fs/promises';
import { dirname } from 'node:path';

export class FileStorage {
  constructor(filename) { this.filename = filename; this.serial = Promise.resolve(); }
  transaction(body) {
    const job = this.serial.then(async () => {
      let values;
      try { values = JSON.parse(await readFile(this.filename, 'utf8')); }
      catch (error) { if (error.code === 'ENOENT') values = {}; else throw error; }
      const result = await body({ get: async key => structuredClone(values[key]), put: async (key, value) => { values[key] = structuredClone(value); } });
      await mkdir(dirname(this.filename), { recursive: true, mode: 0o700 });
      const temporary = this.filename + '.tmp';
      const file = await open(temporary, 'w', 0o600);
      try { await file.writeFile(JSON.stringify(values)); await file.sync(); }
      finally { await file.close(); }
      await rename(temporary, this.filename);
      return result;
    });
    this.serial = job.catch(() => {});
    return job;
  }
}
