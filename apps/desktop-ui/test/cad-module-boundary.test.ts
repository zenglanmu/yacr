import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';

// Source structure only: this does not mount the UI or verify runtime pixels.
test('the CAD affordance module does not re-export other modules', async () => {
  const source = await readModule('cad/cad.ts');
  assert.doesNotMatch(
    source,
    /\bexport\s+(?:type\s+)?(?:\*(?:\s+as\s+\w+)?|\{[^}]*\})\s+from\s*['"]/,
  );
});
