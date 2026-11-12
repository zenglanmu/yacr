// QuickCalc's arithmetic engine — the honest calculator.
//
// What the dialog actually offers: numbers, `+ - * / ^`, parentheses, a decimal
// point, a sign flip, clear, backspace, memory (M+/MR/MC), copying the result,
// the constants `pi` / `e`, and one-argument functions always written as
// `name(expr)` — `sin cos tan asin acos atan ln log sqrt abs`. Trigonometry has a
// degree/radian switch (degrees by default), because a bare `sin(30)` meaning
// 30 radians would be a trap. There is **no** matrix/engineering suite, no unit
// conversion and no implicit multiplication: `2pi` and `2(3)` are errors, not
// guesses. AutoCALC features this build lacks are not pretended otherwise.
//
// The engine is pure (no React, no DOM), so the contract tests can pin operator
// precedence, power associativity, angle conversion, domain rejection and every
// memory transition.
//
// Model: `expression` is the text being typed (what the input shows); `value` is
// the last evaluated result (what a fresh digit starts from); `justEvaluated`
// decides whether the next digit starts a new expression. `error` is set only by
// an evaluation that cannot produce a finite number — the expression is kept so
// the user can repair it. Undefined mathematics (division by zero, `ln(0)`,
// `sqrt(-1)`, `asin(2)`, `tan` at 90°) is reported as `null`, never as `NaN`,
// `Infinity` or a made-up number.

/** Which unit the trigonometric keys read and produce. */
export type AngleMode = 'deg' | 'rad';

/** The calculator's persistent session state. */
export interface CalcState {
  /** The expression currently being typed (empty after a successful `=`). */
  readonly expression: string;
  /** The last evaluated result, formatted (empty before the first `=`). */
  readonly value: string;
  /** The memory register (0 means empty). */
  readonly memory: number;
  /** The last evaluation failed; the expression is kept for repair. */
  readonly error: boolean;
  /** The last action was `=`; the next digit starts a new expression. */
  readonly justEvaluated: boolean;
  /** The trigonometric unit mode (degrees unless the user switches). */
  readonly angle: AngleMode;
}

/** The startup state: empty expression, empty memory, degrees. */
export const INITIAL_CALC_STATE: CalcState = Object.freeze({
  expression: '',
  value: '',
  memory: 0,
  error: false,
  justEvaluated: false,
  angle: 'deg',
});

/** Which operand transform a unary key applies. */
export type CalcUnaryKind = 'square' | 'inverse';

/** The number shown as the main line (expression, then last result, then `0`). */
export function calcDisplay(state: CalcState): string {
  if (state.expression !== '') return state.expression;
  return state.value !== '' ? state.value : '0';
}

/** The live result preview for a complete expression, or `null`. */
export function calcPreview(state: CalcState): string | null {
  if (state.error) return null;
  if (state.expression.trim() === '') return null;
  const result = evaluateExpression(state.expression, state.angle);
  return result === null ? null : formatCalcNumber(result);
}

/** The text "copy result" copies: the evaluated expression, else the last value. */
export function calcCopyText(state: CalcState): string {
  const fromExpression = evaluateExpression(state.expression, state.angle);
  if (fromExpression !== null) return formatCalcNumber(fromExpression);
  if (state.value !== '') return state.value;
  return calcDisplay(state);
}

/** Whether the memory register holds a non-zero value (the `M` indicator). */
export function calcMemoryActive(state: CalcState): boolean {
  return state.memory !== 0;
}

// --- Evaluation -----------------------------------------------------------

/** Format a finite number without float noise (`0.30000000000000004` → `0.3`). */
export function formatCalcNumber(value: number): string {
  if (!Number.isFinite(value)) return '';
  const rounded = Number(value.toPrecision(12));
  return Object.is(rounded, -0) ? '0' : String(rounded);
}

type Operator = '+' | '-' | '*' | '/' | '^' | '(' | ')';

interface IdentToken {
  readonly name: string;
}

type Token = number | Operator | IdentToken;

const NUMBER_RE = /^(?:\d+(?:\.\d*)?|\.\d+)/;
const IDENT_RE = /^[A-Za-z]+/;

/**
 * Tokenize `text`, or `null` when it contains anything the calculator cannot read.
 *
 * Identifiers are lower-cased for lookup only (the typed text keeps its case).
 * There is no implicit multiplication, so `2pi` tokenizes as two tokens and the
 * parser rejects the missing operator.
 */
function tokenize(text: string): Token[] | null {
  const input = String(text ?? '');
  const tokens: Token[] = [];
  let index = 0;
  while (index < input.length) {
    const char = input[index];
    if (/\s/.test(char)) { index += 1; continue; }
    if (/[()+\-*/^]/.test(char)) {
      tokens.push(char as Operator);
      index += 1;
      continue;
    }
    const rest = input.slice(index);
    const number = NUMBER_RE.exec(rest);
    if (number) {
      tokens.push(Number(number[0]));
      index += number[0].length;
      continue;
    }
    const ident = IDENT_RE.exec(rest);
    if (ident) {
      tokens.push({ name: ident[0].toLowerCase() });
      index += ident[0].length;
      continue;
    }
    return null;
  }
  return tokens;
}

const CONSTANTS = new Map<string, number>([
  ['pi', Math.PI],
  ['e', Math.E],
]);

/** Degrees are only a presentation unit: functions convert, then convert back. */
function toRadians(value: number, angle: AngleMode): number {
  return angle === 'deg' ? value * (Math.PI / 180) : value;
}

function fromRadians(value: number, angle: AngleMode): number {
  return angle === 'deg' ? value * (180 / Math.PI) : value;
}

type CalcFunction = (argument: number, angle: AngleMode) => number;

const CALC_FUNCTIONS: Record<string, CalcFunction> = {
  sin: (argument, angle) => Math.sin(toRadians(argument, angle)),
  cos: (argument, angle) => Math.cos(toRadians(argument, angle)),
  // tan has no value at 90° (+ k·180°); the cosine of the converted argument is
  // what would divide, so a cosine this close to zero is reported as undefined
  // instead of an enormous float that only looks like a number.
  tan: (argument, angle) => {
    const radians = toRadians(argument, angle);
    return Math.abs(Math.cos(radians)) < 1e-12 ? Number.NaN : Math.tan(radians);
  },
  asin: (argument, angle) => fromRadians(Math.asin(argument), angle),
  acos: (argument, angle) => fromRadians(Math.acos(argument), angle),
  atan: (argument, angle) => fromRadians(Math.atan(argument), angle),
  ln: argument => Math.log(argument),
  log: argument => Math.log10(argument),
  sqrt: argument => Math.sqrt(argument),
  abs: argument => Math.abs(argument),
};

/**
 * Evaluate an infix expression with normal precedence.
 *
 * Grammar (tighter at the top):
 *
 *   expression := term (('+' | '-') term)*
 *   term       := unary (('*' | '/') unary)*
 *   unary      := ('+' | '-') unary | power
 *   power      := primary ('^' unary)?        // right-associative
 *   primary    := number | '(' expression ')' | constant | function '(' expression ')'
 *
 * `unary` calls `power`, so `-2^2` is `-(2^2) = -4`; `power` takes `unary` on the
 * right, so `2^3^2` is `2^(3^2) = 512` and `2^-1` is `0.5`.
 *
 * Returns `null` for a malformed expression, a division by zero, an out-of-domain
 * function (`ln(0)`, `sqrt(-1)`, `asin(2)`, `tan` at 90°), or any non-finite
 * result — never a fabricated number. Empty input is `null` too.
 */
export function evaluateExpression(text: string, angle: AngleMode = 'deg'): number | null {
  const tokens = tokenize(text);
  if (!tokens || tokens.length === 0) return null;
  let at = 0;

  function push(value: number): number | null {
    return Number.isFinite(value) ? value : null;
  }

  function primary(): number | null {
    const token = tokens![at];
    if (token === undefined) return null;
    if (typeof token === 'number') {
      at += 1;
      return token;
    }
    if (typeof token === 'string') {
      if (token !== '(') return null;
      at += 1;
      const inner = expression();
      if (inner === null || tokens![at] !== ')') return null;
      at += 1;
      return inner;
    }
    at += 1;
    const constant = CONSTANTS.get(token.name);
    if (constant !== undefined) return constant;
    const fn = CALC_FUNCTIONS[token.name];
    // A function name is only a value when it is called: bare `sin` is an error.
    if (!fn) return null;
    if (tokens![at] !== '(') return null;
    at += 1;
    const argument = expression();
    if (argument === null || tokens![at] !== ')') return null;
    at += 1;
    return push(fn(argument, angle));
  }

  function power(): number | null {
    const base = primary();
    if (base === null) return null;
    if (tokens![at] !== '^') return base;
    at += 1;
    const exponent = unary();
    if (exponent === null) return null;
    return push(Math.pow(base, exponent));
  }

  function unary(): number | null {
    const token = tokens![at];
    if (token === '+' || token === '-') {
      at += 1;
      const operand = unary();
      if (operand === null) return null;
      return token === '-' ? -operand : operand;
    }
    return power();
  }

  function term(): number | null {
    let left = unary();
    if (left === null) return null;
    while (tokens![at] === '*' || tokens![at] === '/') {
      const operator = tokens![at];
      at += 1;
      const right = unary();
      if (right === null) return null;
      const next = operator === '*' ? left * right : left / right;
      left = push(next);
      if (left === null) return null;
    }
    return left;
  }

  function expression(): number | null {
    let left = term();
    if (left === null) return null;
    while (tokens![at] === '+' || tokens![at] === '-') {
      const operator = tokens![at];
      at += 1;
      const right = term();
      if (right === null) return null;
      const next = operator === '+' ? left + right : left - right;
      left = push(next);
      if (left === null) return null;
    }
    return left;
  }

  const result = expression();
  return result !== null && at === tokens.length ? result : null;
}

// --- Transitions ----------------------------------------------------------

const ALLOWED_INPUT_RE = /[^0-9A-Za-z.+\-*/()^ \t]/g;

/**
 * Keep only the characters the calculator can read (typing into the field).
 *
 * Letters are kept because function names and constants are spelled out; the
 * text is **not** lower-cased here (the tokenizer is case-insensitive).
 */
export function setCalcExpression(state: CalcState, text: unknown): CalcState {
  const expression = String(text ?? '').replace(ALLOWED_INPUT_RE, '');
  return { ...state, expression, error: false, justEvaluated: false };
}

const SINGLE_CHAR_TOKENS_RE = /^[0-9.+\-*/()^]$/;

const FUNCTION_TOKENS: readonly string[] = [
  'sin(', 'cos(', 'tan(', 'asin(', 'acos(', 'atan(', 'ln(', 'log(', 'sqrt(', 'abs(',
];

const KEY_TOKENS = new Set<string>(['pi', 'e', ...FUNCTION_TOKENS]);

/** Tokens that begin a fresh expression after `=` (instead of continuing its value). */
const FRESH_TOKENS = new Set<string>(['pi', 'e', ...FUNCTION_TOKENS]);

function isAcceptedToken(token: string): boolean {
  return SINGLE_CHAR_TOKENS_RE.test(token) || KEY_TOKENS.has(token);
}

/**
 * Append one keypad token.
 *
 * Accepted tokens are the digits, `.` and the operators `+ - * / ( ) ^`, plus the
 * raw lowercase spellings `pi`, `e`, `sin(`, `cos(`, `tan(`, `asin(`, `acos(`,
 * `atan(`, `ln(`, `log(`, `sqrt(`, `abs(`. Anything else is ignored, never
 * inserted.
 *
 * After `=`, a digit / decimal point / `(` / `pi` / `e` / a function head starts
 * a new expression; an operator continues from the result.
 */
export function appendCalcToken(state: CalcState, token: string): CalcState {
  if (!isAcceptedToken(token)) return state;
  const startsFresh = state.justEvaluated
    && (/[0-9.(]/.test(token) || FRESH_TOKENS.has(token));
  const base = state.justEvaluated
    ? (startsFresh ? '' : state.value)
    : state.expression;
  return { ...state, expression: base + token, error: false, justEvaluated: false };
}

/** Remove the last character (the result becomes editable after `=`). */
export function backspaceCalc(state: CalcState): CalcState {
  const base = state.expression !== '' ? state.expression : state.value;
  return { ...state, expression: base.slice(0, -1), value: '', error: false, justEvaluated: false };
}

/** Clear the entry. Memory and the angle mode survive (that is what `MC` is for). */
export function clearCalc(state: CalcState): CalcState {
  return { ...state, expression: '', value: '', error: false, justEvaluated: false };
}

/**
 * Apply a unary key to the current operand (the typed expression, else the last
 * result).
 *
 * `square` appends `^2`; `inverse` wraps the operand as `1/(…)`. With neither an
 * expression nor a value there is nothing to transform, so the state is returned
 * untouched rather than inventing a `0`.
 */
export function applyCalcUnary(state: CalcState, kind: CalcUnaryKind): CalcState {
  const expression = state.expression !== '' ? state.expression : state.value;
  if (expression === '') return state;
  const next = kind === 'square' ? `${expression}^2` : `1/(${expression})`;
  return { ...state, expression: next, value: '', error: false, justEvaluated: false };
}

/** Switch the trigonometric unit mode (`DEG`/`RAD`). */
export function setCalcAngle(state: CalcState, mode: AngleMode): CalcState {
  return { ...state, angle: mode };
}

/** Flip degrees ↔ radians. */
export function toggleCalcAngle(state: CalcState): CalcState {
  return { ...state, angle: state.angle === 'deg' ? 'rad' : 'deg' };
}

const TRAILING_NUMBER_RE = /(\d+(?:\.\d*)?|\.\d+)$/;
const WRAPPED_NEGATIVE_RE = /\(-(\d+(?:\.\d*)?|\.\d+)\)$/;

/** Flip the sign of the number at the end of the expression (a visible `(-n)`). */
export function toggleSignCalc(state: CalcState): CalcState {
  const expression = state.expression !== '' ? state.expression : state.value;
  const wrapped = WRAPPED_NEGATIVE_RE.exec(expression);
  let next: string;
  if (wrapped) {
    next = expression.slice(0, wrapped.index) + wrapped[1];
  } else {
    const trailing = TRAILING_NUMBER_RE.exec(expression);
    next = trailing
      ? `${expression.slice(0, trailing.index)}(-${trailing[1]})`
      : (/[+\-*/(]$/.test(expression) || expression === '' ? `${expression}-` : expression);
  }
  return { ...state, expression: next, value: '', error: false, justEvaluated: false };
}

/**
 * Evaluate the expression into `value`.
 *
 * A malformed expression or a non-finite result (including division by zero and
 * an out-of-domain function) sets `error` and keeps the expression untouched: the
 * dialog never shows a made-up number.
 */
export function evaluateCalc(state: CalcState): CalcState {
  const expression = state.expression.trim() !== '' ? state.expression : state.value;
  const result = evaluateExpression(expression, state.angle);
  if (result === null) {
    return { ...state, error: true, justEvaluated: false };
  }
  return {
    ...state,
    expression: '',
    value: formatCalcNumber(result),
    error: false,
    justEvaluated: true,
  };
}

function currentNumber(state: CalcState): number | null {
  const fromExpression = evaluateExpression(state.expression, state.angle);
  if (fromExpression !== null) return fromExpression;
  if (state.value !== '') {
    const value = Number(state.value);
    return Number.isFinite(value) ? value : null;
  }
  return null;
}

/** Add the current result to memory (`M+`). No number is a quiet no-op. */
export function memoryAddCalc(state: CalcState): CalcState {
  const current = currentNumber(state);
  return current === null ? state : { ...state, memory: state.memory + current };
}

/** Replace the entry with the memory value (`MR`). */
export function memoryRecallCalc(state: CalcState): CalcState {
  return {
    ...state,
    expression: formatCalcNumber(state.memory),
    value: '',
    error: false,
    justEvaluated: false,
  };
}

/** Clear memory (`MC`). */
export function memoryClearCalc(state: CalcState): CalcState {
  return { ...state, memory: 0 };
}
