// QuickCalc's arithmetic engine — the honest four-function calculator.
//
// Only what the dialog actually offers: numbers, `+ - * /`, parentheses, a
// decimal point, a sign flip, clear, backspace, memory (M+/MR/MC) and copying the
// result. There are **no** engineering/trigonometric/matrix functions: AutoCAD's
// QuickCalc has them, this build does not, and nothing here pretends otherwise.
//
// The engine is pure (no React, no DOM), so the contract tests can pin operator
// precedence, parentheses, division by zero and every memory transition.
//
// Model: `expression` is the text being typed (what the input shows); `value` is
// the last evaluated result (what a fresh digit starts from); `justEvaluated`
// decides whether the next digit starts a new expression. `error` is set only by
// an evaluation that cannot produce a finite number — the expression is kept so
// the user can repair it.

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
}

/** The startup state: empty expression, empty memory. */
export const INITIAL_CALC_STATE: CalcState = Object.freeze({
  expression: '',
  value: '',
  memory: 0,
  error: false,
  justEvaluated: false,
});

/** The number shown as the main line (expression, then last result, then `0`). */
export function calcDisplay(state: CalcState): string {
  if (state.expression !== '') return state.expression;
  return state.value !== '' ? state.value : '0';
}

/** The live result preview for a complete expression, or `null`. */
export function calcPreview(state: CalcState): string | null {
  if (state.error) return null;
  if (state.expression.trim() === '') return null;
  const result = evaluateExpression(state.expression);
  return result === null ? null : formatCalcNumber(result);
}

/** The text "copy result" copies: the evaluated expression, else the last value. */
export function calcCopyText(state: CalcState): string {
  const fromExpression = evaluateExpression(state.expression);
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

type Token = number | '+' | '-' | '*' | '/' | '(' | ')';

const TOKEN_RE = /\s*(\d+(?:\.\d*)?|\.\d+|[()+\-*/])/y;

/** Tokenize `text`, or `null` when it contains anything the calculator cannot read. */
function tokenize(text: string): Token[] | null {
  const input = String(text ?? '');
  const tokens: Token[] = [];
  let index = 0;
  while (index < input.length) {
    if (/\s/.test(input[index])) { index += 1; continue; }
    TOKEN_RE.lastIndex = index;
    const match = TOKEN_RE.exec(input);
    if (!match || match.index !== index) return null;
    const raw = match[1];
    tokens.push(/^[()+\-*/]$/.test(raw) ? raw as Token : Number(raw));
    index = TOKEN_RE.lastIndex;
  }
  return tokens;
}

/**
 * Evaluate an infix expression with normal precedence.
 *
 * Returns `null` for a malformed expression, a division by zero, or any
 * non-finite result — never a fabricated number. Empty input is `null` too.
 */
export function evaluateExpression(text: string): number | null {
  const tokens = tokenize(text);
  if (!tokens || tokens.length === 0) return null;
  let at = 0;

  function push(value: number): number | null {
    return Number.isFinite(value) ? value : null;
  }

  function factor(): number | null {
    const token = tokens![at];
    if (token === undefined) return null;
    if (token === '+' || token === '-') {
      at += 1;
      const operand = factor();
      if (operand === null) return null;
      return token === '-' ? -operand : operand;
    }
    if (token === '(') {
      at += 1;
      const inner = expression();
      if (inner === null || tokens![at] !== ')') return null;
      at += 1;
      return inner;
    }
    if (typeof token === 'number') {
      at += 1;
      return token;
    }
    return null;
  }

  function term(): number | null {
    let left = factor();
    if (left === null) return null;
    while (tokens![at] === '*' || tokens![at] === '/') {
      const operator = tokens![at];
      at += 1;
      const right = factor();
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

const ALLOWED_INPUT_RE = /[^0-9.+\-*/()\s]/g;

/** Keep only the characters the calculator can read (typing into the field). */
export function setCalcExpression(state: CalcState, text: unknown): CalcState {
  const expression = String(text ?? '').replace(ALLOWED_INPUT_RE, '');
  return { ...state, expression, error: false, justEvaluated: false };
}

const TOKEN_RE_INPUT = /^(\d|\.|\+|-|\*|\/|\(|\))$/;

/**
 * Append one keypad token.
 *
 * After `=`, a digit (or a decimal point / `(`) starts a new expression; an
 * operator continues from the result. Unknown tokens are ignored, never inserted.
 */
export function appendCalcToken(state: CalcState, token: string): CalcState {
  if (!TOKEN_RE_INPUT.test(token)) return state;
  const startsFresh = state.justEvaluated && /[0-9.(]/.test(token);
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

/** Clear the entry. Memory survives (that is what `MC` is for). */
export function clearCalc(state: CalcState): CalcState {
  return { ...state, expression: '', value: '', error: false, justEvaluated: false };
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
 * A malformed expression or a non-finite result (including division by zero)
 * sets `error` and keeps the expression untouched: the dialog never shows a
 * made-up number.
 */
export function evaluateCalc(state: CalcState): CalcState {
  const expression = state.expression.trim() !== '' ? state.expression : state.value;
  const result = evaluateExpression(expression);
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
  const fromExpression = evaluateExpression(state.expression);
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
