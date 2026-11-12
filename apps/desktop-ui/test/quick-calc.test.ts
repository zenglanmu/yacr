// Contract tests for QuickCalc's arithmetic engine (`src/chrome/quick-calc.ts`).
//
// This is a **behavioural** suite: it imports the pure module directly, so the
// assertions are about values, not source text. Four reader-facing promises are
// pinned here:
//
//   * ordinary arithmetic keeps its precedence, and `^` is right-associative and
//     binds tighter than unary minus (`-2^2 === -4`, `2^3^2 === 512`);
//   * the scientific layer is honest — constants, the ten functions, the
//     degree/radian switch, and `null` (never `NaN`/`Infinity`) for anything
//     undefined or malformed (`ln(0)`, `sqrt(-1)`, `2pi`, `sin 30`);
//   * keypad insertion accepts exactly the documented tokens and ignores the
//     rest, including the old `pow` token;
//   * the new angle mode is sticky: clear/backspace/memory/evaluate keep it.
//
// Like the rest of `test/`, this file is `.ts` by name only: the body is plain
// JavaScript and `tsc` never checks it (`tsconfig.json` includes `src` only).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  INITIAL_CALC_STATE,
  appendCalcToken,
  applyCalcUnary,
  backspaceCalc,
  calcCopyText,
  calcDisplay,
  calcMemoryActive,
  calcPreview,
  clearCalc,
  evaluateCalc,
  evaluateExpression,
  formatCalcNumber,
  memoryAddCalc,
  memoryClearCalc,
  memoryRecallCalc,
  setCalcAngle,
  setCalcExpression,
  toggleCalcAngle,
  toggleSignCalc,
} from '../src/chrome/quick-calc.ts';

/** Float comparison with an explicit tolerance (trig results are not exact). */
function close(actual, expected, tolerance = 1e-9, message = '') {
  assert.ok(
    typeof actual === 'number' && Math.abs(actual - expected) <= tolerance,
    `${message} expected ~${expected}, got ${String(actual)}`,
  );
}

function typing(tokens, state = INITIAL_CALC_STATE) {
  return tokens.reduce((next, token) => appendCalcToken(next, token), state);
}

test('four-function arithmetic keeps normal precedence and parentheses', () => {
  assert.equal(evaluateExpression('2+3*4'), 14);
  assert.equal(evaluateExpression('(2+3)*4'), 20);
  assert.equal(evaluateExpression('-3*2'), -6);
  assert.equal(evaluateExpression('10/4'), 2.5);
  assert.equal(evaluateExpression('2*(3+(4-1))'), 12);
  assert.equal(evaluateExpression('  6 / 3 + 1  '), 3, 'whitespace anywhere is fine');
  assert.equal(evaluateExpression('1/0'), null, 'division by zero never fabricates a number');
  assert.equal(evaluateExpression('0/0'), null);
});

test('power is right-associative and binds tighter than unary minus', () => {
  assert.equal(evaluateExpression('2^3'), 8);
  assert.equal(evaluateExpression('-2^2'), -4);
  assert.equal(evaluateExpression('(-2)^2'), 4);
  assert.equal(evaluateExpression('2^3^2'), 512);
  assert.equal(evaluateExpression('2^-1'), 0.5);
  assert.equal(evaluateExpression('2*3^2'), 18, 'power beats multiplication');
  assert.equal(evaluateExpression('4^0.5'), 2);
  assert.equal(evaluateExpression('9^999'), null, 'overflow is not a number we show');
});

test('constants are read case-insensitively, as numbers and not notation', () => {
  close(evaluateExpression('pi'), Math.PI, 1e-12);
  close(evaluateExpression('PI'), Math.PI, 1e-12);
  close(evaluateExpression('e'), Math.E, 1e-12);
  close(evaluateExpression('E'), Math.E, 1e-12);
  close(evaluateExpression('2*pi'), Math.PI * 2, 1e-12);
  assert.equal(evaluateExpression('3e2'), null, '`e` is Euler\'s number, not 3×10²');
});

test('each scientific function is wired, with parentheses required', () => {
  close(evaluateExpression('sin(30)'), 0.5, 1e-12);
  close(evaluateExpression('cos(60)'), 0.5, 1e-12);
  close(evaluateExpression('tan(45)'), 1, 1e-12);
  close(evaluateExpression('asin(0.5)'), 30, 1e-6);
  close(evaluateExpression('acos(0.5)'), 60, 1e-6);
  close(evaluateExpression('atan(1)'), 45, 1e-6);
  close(evaluateExpression('ln(e)'), 1, 1e-12);
  close(evaluateExpression('log(100)'), 2, 1e-12);
  close(evaluateExpression('sqrt(9)'), 3, 1e-12);
  close(evaluateExpression('abs(-4)'), 4, 1e-12);
  close(evaluateExpression('sin(pi/2)', 'rad'), 1, 1e-9, 'expressions nest inside a call');
  close(evaluateExpression('sqrt(abs(-16))'), 4, 1e-12, 'calls nest inside calls');
  // No implicit multiplication and no bare function names.
  assert.equal(evaluateExpression('sin 30'), null);
  assert.equal(evaluateExpression('sin'), null);
  assert.equal(evaluateExpression('2pi'), null);
  assert.equal(evaluateExpression('2(3)'), null);
  assert.equal(evaluateExpression('pow(2,3)'), null, 'there is no `pow` key');
});

test('deg is the default for trig input and inverse-trig output; rad is plain JS', () => {
  close(evaluateExpression('sin(30)'), 0.5, 1e-12, 'default argument is deg');
  close(evaluateExpression('sin(30)', 'deg'), 0.5, 1e-12);
  close(evaluateExpression('sin(30)', 'rad'), Math.sin(30), 1e-12);
  close(evaluateExpression('asin(0.5)', 'deg'), 30, 1e-6);
  close(evaluateExpression('asin(0.5)', 'rad'), Math.asin(0.5), 1e-12);
  close(evaluateExpression('tan(pi/4)', 'rad'), 1, 1e-12);
  close(evaluateExpression('atan(1)', 'rad'), Math.PI / 4, 1e-12);
});

test('undefined mathematics returns null instead of NaN or Infinity', () => {
  for (const source of [
    'ln(0)', 'ln(-1)', 'log(0)', 'log(-5)', 'sqrt(-1)',
    'asin(2)', 'acos(-2)', 'tan(90)', 'tan(270)',
    '(-8)^0.5', '0^-1', '1/0', '-1/0', 'sqrt(-1)*0',
  ]) {
    assert.equal(evaluateExpression(source), null, `${source} has no finite value`);
  }
  assert.equal(evaluateExpression('tan(pi/2)', 'rad'), null, 'π/2 in radians is undefined too');
});

test('malformed input is rejected, never guessed at', () => {
  for (const source of ['', '   ', '2+', '()', '(2+3', '2++*3', 'sin 30', '2pi', '2 3', '.', '*2', 'pi)', 'sqrt()']) {
    assert.equal(evaluateExpression(source), null, `${JSON.stringify(source)} is not an expression`);
  }
  assert.equal(evaluateExpression(undefined), null, 'missing text is not zero');
});

test('the keypad accepts exactly the documented tokens', () => {
  for (const token of ['0', '7', '.', '+', '-', '*', '/', '(', ')', '^', 'pi', 'e']) {
    assert.equal(appendCalcToken(INITIAL_CALC_STATE, token).expression, token);
  }
  for (const token of ['sin(', 'cos(', 'tan(', 'asin(', 'acos(', 'atan(', 'ln(', 'log(', 'sqrt(', 'abs(']) {
    assert.equal(appendCalcToken(INITIAL_CALC_STATE, token).expression, token);
  }
  // Inserted text is raw lowercase, and the keypad tokens are exact.
  assert.equal(appendCalcToken(INITIAL_CALC_STATE, 'PI').expression, '');
  assert.equal(appendCalcToken(INITIAL_CALC_STATE, 'Sin(').expression, '');
  assert.equal(appendCalcToken(INITIAL_CALC_STATE, 'sin').expression, '');
  assert.equal(appendCalcToken(INITIAL_CALC_STATE, 'pow').expression, '');
  assert.equal(appendCalcToken(INITIAL_CALC_STATE, 'sin(x)').expression, '');
  assert.equal(appendCalcToken(INITIAL_CALC_STATE, '7').expression, '7');
});

test('after `=` a value token starts fresh, an operator continues the result', () => {
  let state = typing(['2', '^', '3']);
  state = evaluateCalc(state);
  assert.equal(state.value, '8');
  assert.equal(state.expression, '');
  assert.equal(state.justEvaluated, true);

  assert.equal(appendCalcToken(state, '7').expression, '7');
  assert.equal(appendCalcToken(state, '.').expression, '.');
  assert.equal(appendCalcToken(state, '(').expression, '(');
  assert.equal(appendCalcToken(state, 'pi').expression, 'pi');
  assert.equal(appendCalcToken(state, 'e').expression, 'e');
  assert.equal(appendCalcToken(state, 'sqrt(').expression, 'sqrt(');
  assert.equal(appendCalcToken(state, '+').expression, '8+');
  assert.equal(appendCalcToken(state, '*').expression, '8*');
  assert.equal(appendCalcToken(state, '^').expression, '8^');
  assert.equal(appendCalcToken(appendCalcToken(state, '^'), '2').expression, '8^2');
});

test('square and inverse transform the current operand', () => {
  const typed = typing(['5']);
  const squared = applyCalcUnary(typed, 'square');
  assert.equal(squared.expression, '5^2');
  assert.equal(squared.value, '');
  assert.equal(squared.error, false);
  assert.equal(squared.justEvaluated, false);
  assert.equal(evaluateCalc(squared).value, '25');

  const inverted = applyCalcUnary(typed, 'inverse');
  assert.equal(inverted.expression, '1/(5)');
  assert.equal(evaluateCalc(inverted).value, '0.2');

  // With no expression, the last result is the operand.
  const evaluated = { ...INITIAL_CALC_STATE, value: '14', justEvaluated: true };
  assert.equal(applyCalcUnary(evaluated, 'square').expression, '14^2');
  assert.equal(applyCalcUnary(evaluated, 'inverse').expression, '1/(14)');

  // Nothing to operate on: unchanged, not an invented zero.
  assert.equal(applyCalcUnary(INITIAL_CALC_STATE, 'square'), INITIAL_CALC_STATE);
  assert.equal(applyCalcUnary(INITIAL_CALC_STATE, 'inverse'), INITIAL_CALC_STATE);
});

test('the angle mode toggles and survives every state transition', () => {
  assert.equal(INITIAL_CALC_STATE.angle, 'deg');
  assert.equal(toggleCalcAngle(INITIAL_CALC_STATE).angle, 'rad');
  assert.equal(toggleCalcAngle(toggleCalcAngle(INITIAL_CALC_STATE)).angle, 'deg');
  assert.equal(setCalcAngle(INITIAL_CALC_STATE, 'rad').angle, 'rad');
  assert.equal(setCalcAngle(INITIAL_CALC_STATE, 'rad').expression, '');

  const radians = setCalcAngle(INITIAL_CALC_STATE, 'rad');
  const typed = typing(['sin(', 'pi', '/', '2', ')'], radians);
  assert.equal(evaluateCalc(typed).value, '1', 'the engine reads the state angle');
  assert.equal(calcPreview(typed), '1');
  assert.equal(calcCopyText(typed), '1');
  assert.equal(evaluateCalc(typed).angle, 'rad');

  assert.equal(clearCalc(typed).angle, 'rad');
  assert.equal(backspaceCalc(typed).angle, 'rad');
  assert.equal(toggleSignCalc(typed).angle, 'rad');
  assert.equal(setCalcExpression(radians, '2+2').angle, 'rad');
  assert.equal(memoryAddCalc(typed).angle, 'rad');
  assert.equal(memoryRecallCalc(typed).angle, 'rad');
  assert.equal(memoryClearCalc(typed).angle, 'rad');
  assert.equal(applyCalcUnary(typed, 'square').angle, 'rad');

  const failed = evaluateCalc({ ...radians, expression: 'ln(0)' });
  assert.equal(failed.error, true);
  assert.equal(failed.angle, 'rad');
});

test('state transitions keep their documented shape', () => {
  let state = typing(['2', '+', '3']);
  state = evaluateCalc(state);
  assert.equal(state.value, '5');
  assert.equal(state.expression, '');
  assert.equal(state.justEvaluated, true);

  // Backspace edits the typed expression, and clears the carried result text
  // one character at a time (an existing behaviour, pinned here so it stays).
  const editing = backspaceCalc(typing(['1', '2', '+', '3']));
  assert.equal(editing.expression, '12+');
  assert.equal(editing.value, '');
  assert.equal(backspaceCalc(state).expression, '', 'the result text is the base after `=`');

  // Memory arithmetic goes through the same evaluator.
  let memory = typing(['2', '+', '3']);
  memory = memoryAddCalc(memory);
  assert.equal(memory.memory, 5);
  assert.equal(calcMemoryActive(memory), true);
  memory = memoryRecallCalc(memory);
  assert.equal(memory.expression, '5');
  memory = memoryClearCalc(memory);
  assert.equal(memory.memory, 0);
  assert.equal(calcMemoryActive(memory), false);

  // Sign flip is still the visible `(-n)` form.
  assert.equal(toggleSignCalc(typing(['5'])).expression, '(-5)');

  // A failed evaluation keeps the expression and reports `error`.
  const failed = evaluateCalc({ ...INITIAL_CALC_STATE, expression: 'sqrt(-1)' });
  assert.equal(failed.error, true);
  assert.equal(failed.expression, 'sqrt(-1)');
  assert.equal(calcPreview(failed), null);

  // Typing keeps the display coherent.
  assert.equal(calcDisplay(INITIAL_CALC_STATE), '0');
  assert.equal(calcDisplay(typing(['1', '+', '2'])), '1+2');
  assert.equal(setCalcExpression(INITIAL_CALC_STATE, 'sin(30)+2').expression, 'sin(30)+2');
  assert.equal(setCalcExpression(INITIAL_CALC_STATE, 'sin(30)$%').expression, 'sin(30)');
});

test('number formatting rounds float noise and never prints negative zero', () => {
  assert.equal(formatCalcNumber(0.1 + 0.2), '0.3');
  assert.equal(formatCalcNumber(-0), '0');
  assert.equal(formatCalcNumber(Number.NEGATIVE_INFINITY), '');
  assert.equal(formatCalcNumber(Number.NaN), '');
  const zero = evaluateExpression('-0');
  assert.notEqual(zero, null);
  assert.equal(formatCalcNumber(zero), '0');
});
