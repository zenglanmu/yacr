// AutoCAD-style command aliases in the chrome: storage, lookup and the
// suggestion merge.
//
// The alias table comes from the host (`cad_command_aliases`), which derives it
// from the same Rust `COMMAND_ALIASES` table the router normalizes against. These
// tests pin the two halves a frontend owns: what it does with that answer, and
// that an alias is a **suggestion** — admission still runs through the existing
// `admitCommandText` path (the host canonicalises the typed text).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  aliasTarget,
  commandAliases,
  defaultSuggestionIndex,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandAliases,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { HOST_NAMES, readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

// The host wire shape: `{ alias, command }` (camelCase, `command` is canonical).
const HOST_ALIASES = [
  { alias: 'L', command: 'LINE' },
  { alias: 'C', command: 'CIRCLE' },
  { alias: 'CO', command: 'COPY' },
  { alias: 'CP', command: 'COPY' },
  { alias: 'Z', command: 'ZOOM' },
  { alias: 'EX', command: 'EXTEND' },
  { alias: 'X', command: 'EXPLODE' },
];

function loadAliases() {
  setCommandAliases(HOST_ALIASES);
}

test('the alias table is replaced, normalized and deduped; empty means "none known"', () => {
  assert.deepEqual(commandAliases(), [], 'no aliases before the host answers');
  const rows = setCommandAliases([
    { alias: ' l ', command: ' line ' },
    // A conflicting duplicate must not replace the first row; a row missing
    // either half is dropped instead of being completed from a guess.
    { alias: 'L', command: 'CIRCLE' },
    { alias: '', command: 'LINE' },
    { alias: 'Q', command: '' },
    { alias: 'CO', command: 'COPY' },
    null,
  ]);
  assert.deepEqual(rows, [
    { alias: 'L', target: 'LINE' },
    { alias: 'CO', target: 'COPY' },
  ]);
  assert.deepEqual(commandAliases(), rows);
  // The table is frozen: a caller cannot mutate module state behind the app's
  // back and leave the suggestions and the host disagreeing.
  assert.ok(Object.isFrozen(rows));
  assert.ok(Object.isFrozen(rows[0]));

  // Reset: an empty answer (a failed `cad_command_aliases`) leaves no aliases.
  assert.deepEqual(setCommandAliases([]), []);
  assert.equal(aliasTarget('L'), null);
  assert.deepEqual(setCommandAliases(null), []);
});

test('aliasTarget is case-insensitive, trimmed, and null for anything else', () => {
  loadAliases();
  assert.equal(aliasTarget('L'), 'LINE');
  assert.equal(aliasTarget(' l '), 'LINE');
  assert.equal(aliasTarget('co'), 'COPY');
  assert.equal(aliasTarget('CP'), 'COPY');
  // A canonical name is not an alias: the host normalizes it itself, and the
  // chrome must not present "LINE is a shorthand for LINE".
  assert.equal(aliasTarget('LINE'), null);
  // Unknown, empty, and a real AutoCAD abbreviation the host did not send in this
  // fixture's alias table (`MA` for MATCHPROP is not in `HOST_ALIASES`).
  assert.equal(aliasTarget('NOT_AN_ALIAS'), null);
  assert.equal(aliasTarget('MA'), null);
  assert.equal(aliasTarget(''), null);
  assert.equal(aliasTarget(null), null);
  setCommandAliases([]);
});

test('an alias stays a suggestion: admission still routes through the host vocabulary', () => {
  loadAliases();
  // The host admits aliases (`cad_commands` lists them), so the typed shorthand is
  // admitted — but the admission record names the alias; canonicalisation is the
  // router's job (`cad_command`).
  assert.equal(isSupportedCommand('L'), true);
  assert.deepEqual(admitCommandText('L', { status: loaded }), { kind: 'admitted', command: 'L', reasonKey: null });
  // Without the host vocabulary nothing is admitted, aliases included.
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('L'), false);
  assert.equal(admitCommandText('L', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  setCommandAliases([]);
});

test('suggestions merge alias rows between canonical prefix and mid-string matches', () => {
  loadAliases();
  const rows = filterCommandSuggestions('l');
  const aliasRow = rows.find(row => row.id === 'L');
  assert.ok(aliasRow, 'L must be suggested once the alias table is loaded');
  assert.equal(aliasRow.target, 'LINE', 'the alias entry carries its canonical target');
  assert.equal(aliasRow.supported, true);
  // The canonical spelling keeps its usual row, and the shorthand is listed after
  // it: an exact canonical row is never pushed down by an alias.
  const canonical = rows.findIndex(row => row.id === 'LINE');
  assert.ok(canonical >= 0);
  assert.ok(canonical < rows.indexOf(aliasRow), 'the alias row comes after LINE');
  // ...and a canonical name that merely contains the needle (`PLINE`) does not
  // starve the shorthand of its slot.
  assert.ok(
    !rows.some(row => row.id === 'PLINE') || rows.indexOf(aliasRow) < rows.findIndex(row => row.id === 'PLINE'),
    'the alias row ranks before canonical mid-string matches',
  );
  // Typing the alias exactly highlights its own row (Enter submits `L`, which the
  // host canonicalises), and the row still shows the meaning.
  assert.equal(rows[defaultSuggestionIndex(rows, 'L')].id, 'L');
  // Canonical prefix matches still win: `co` starts with a canonical name.
  const co = filterCommandSuggestions('co');
  assert.equal(co[0].id, 'COPY');
  assert.ok(co.findIndex(row => row.id === 'CO') > co.findIndex(row => row.id === 'COPY'));
  assert.equal(co.find(row => row.id === 'CO').target, 'COPY');
  setCommandAliases([]);
});

test('alias rows match a typed prefix only, and the limit covers the merged list', () => {
  loadAliases();
  // `X` is an alias, so it appears for `x`; a mid-string match on the alias is
  // deliberately not a thing (`EX` contains `X` but is not a prefix match).
  const x = filterCommandSuggestions('x');
  assert.deepEqual(x.filter(row => row.target).map(row => row.id), ['X']);
  const ex = filterCommandSuggestions('ex');
  assert.deepEqual(ex.filter(row => row.target).map(row => row.id), ['EX']);
  // An empty query browses the catalog only — never the shorthand rows.
  assert.equal(filterCommandSuggestions('').some(row => row.target), false);

  // A tiny catalog isolates the merge: canonical rows first, then aliases, and
  // `limit` slices the merged list (so a full canonical list can hide them).
  const catalog = [{ id: 'LINE', labelKey: 'draw.kind.line', icon: 'line' }];
  const aliases = setCommandAliases(HOST_ALIASES);
  assert.deepEqual(
    filterCommandSuggestions('l', { catalog, aliases }).map(row => row.id),
    ['LINE', 'L'],
  );
  assert.deepEqual(
    filterCommandSuggestions('l', { catalog, aliases, limit: 1 }).map(row => row.id),
    ['LINE'],
  );
  // Usage frequency orders the alias rows among themselves, like canonical rows.
  const onlyAliases = [{ id: 'POINT', labelKey: 'draw.kind.point', icon: 'point' }];
  assert.deepEqual(
    filterCommandSuggestions('c', { catalog: onlyAliases, aliases, history: ['CO', 'CO'] }).map(row => row.id),
    ['CO', 'C', 'CP'],
  );
  setCommandAliases([]);
});

test('an empty alias table changes nothing but the extra rows', () => {
  setCommandAliases([]);
  const rows = filterCommandSuggestions('l');
  assert.equal(rows.some(row => row.target), false, 'no shorthand rows without the host answer');
  assert.ok(rows.some(row => row.id === 'LINE'), 'canonical suggestions still work');
  assert.equal(aliasTarget('L'), null);
  // The injected-table form makes "no aliases" explicit and keeps the pure
  // function pure (no module state read when the caller supplies the table).
  assert.deepEqual(
    filterCommandSuggestions('l', { catalog: [{ id: 'LINE', labelKey: 'draw.kind.line', icon: 'line' }], aliases: [] })
      .map(row => row.id),
    ['LINE'],
  );
});

test('the command line renders an alias row as entry + canonical target', async () => {
  const model = await readModule('commands/command-line-model.ts');
  const hook = await readModule('commands/use-command-line.ts');
  const source = await readModule('components/CommandLine.tsx');
  // The host alias table is still the row source. The rules were extracted into
  // the shared controller (both .tsx presentations render from it), so the
  // filter call lives in the model and the table is handed over by the hook.
  assert.match(
    model,
    /filterCommandSuggestions\(command, \{ history, aliases \}\)/,
    'the suggestion list must consume the host alias table',
  );
  assert.ok(hook.includes('aliases: shell.commandAliases as readonly CommandAlias[]'), 'the alias table reaches the filter');
  assert.ok(hook.includes('shell.history, shell.commandAliases]'), 'the alias table is a memo dependency');
  // The alias entry is the id and the canonical name is the row's secondary text;
  // canonical rows keep their translated label. No new classes/markup are added.
  // The row's label key still goes through the translator; TS pins the key's type
  // with an empty-string fallback rather than a cast.
  assert.match(source, /\{suggestion\.target \?\? t\(suggestion\.labelKey(?: \?\? '')?\)\}/);
  assert.ok(source.includes('<span className="completion__id">{suggestion.id}</span>'));
  // The multi-word alias (`ZOOM EXTENTS`) gets a space-free option id, so the
  // `aria-activedescendant` wiring still names the row.
  assert.ok(model.includes("suggestion.id.replace(/\\s+/g, '-')"));
  // The router-does-not-admit badge is unchanged for both row kinds.
  assert.ok(source.includes("t('command.badge.unwired')"));
});

test('the app fetches the alias table once and tolerates a failure', async () => {
  // The fetch lives in `commands/use-host-vocabulary.ts` (extracted from
  // `App.tsx` for the file-size ceiling); the contract is unchanged.
  const source = await readModule('commands/use-host-vocabulary.ts');
  const calls = source.match(/invoke\('cad_command_aliases'\)/g) ?? [];
  assert.equal(calls.length, 1, 'exactly one startup fetch');
  const start = source.indexOf("invoke('cad_command_aliases')");
  const block = source.slice(start, source.indexOf('return () =>', start));
  assert.match(block, /setCommandAliases\(answer as Iterable<CommandAliasInput>\)/, 'the answer replaces the table');
  assert.ok(block.includes('setCommandAliases([])'), 'a failure leaves the table empty');
  // A missing alias table is not a startup error: the command line still admits
  // everything the router knows, it just offers no shorthand rows.
  assert.ok(!block.includes('setError('), 'an alias failure must not surface as an error');
  assert.ok(!block.includes('setCommandCatalog'), 'the alias fetch never touches admission');
});
