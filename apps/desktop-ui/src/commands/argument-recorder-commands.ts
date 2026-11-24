// The ADR-0056 action-recorder argument rows, split out for the code-size gate.
//
// The four predicates live in `argument-record-grammar.ts`; this table only
// attaches them to the seven selectors. The host remains the authority for the
// `.json` destination, the file bridge and every `act*` refusal.
import {
  isActionManagerArgument,
  isActionRecordArgument,
  isActionUserInputArgument,
  isActionUserMessageArgument,
} from './argument-record-grammar.ts';
/** The parser shape `argument-commands.ts` consumes (one typed tail). */
export interface ArgumentSpec { readonly parse: (argument: string) => boolean; }

export const ACTION_RECORDER_ARGUMENT_COMMANDS: Readonly<Record<string, ArgumentSpec>> = Object.freeze({
  ACTRECORD: Object.freeze({ parse: isActionRecordArgument }),
  ACTSTOP: Object.freeze({ parse: isActionRecordArgument }),
  '-ACTSTOP': Object.freeze({ parse: isActionRecordArgument }),
  ACTUSERINPUT: Object.freeze({ parse: isActionUserInputArgument }),
  ACTUSERMESSAGE: Object.freeze({ parse: isActionUserMessageArgument }),
  '-ACTUSERMESSAGE': Object.freeze({ parse: isActionUserMessageArgument }),
  ACTMANAGER: Object.freeze({ parse: isActionManagerArgument }),
});
