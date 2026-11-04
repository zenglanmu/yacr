// Frame-facing value types shared by the decoders.
import type { Point3 } from './bit-reader.ts';

/** Kind 0 = connected lines, 1 = segment pairs; both carry `point_count` points. */
export interface LinesPrimitive { kind: 0 | 1; points: Point3[] }
export interface ArcPrimitive { kind: 2; cx: number; cy: number; cz: number; radius: number; startAngle: number; sweep: number }
export interface CirclePrimitive { kind: 3; cx: number; cy: number; cz: number; radius: number }
export interface TextPrimitive {
  kind: 4;
  origin: Point3;
  height: number;
  rotation: number;
  wrapWidth: number;
  attachment: number;
  text: string;
}
export interface BlockPrimitive {
  kind: 5;
  block: bigint;
  origin: Point3;
  scale: Point3;
  rotation: number;
}
export type Primitive = LinesPrimitive | ArcPrimitive | CirclePrimitive | TextPrimitive | BlockPrimitive;
export interface DecodedEntity { handle: bigint; primitives: Primitive[] }
export interface VendorRecord {
  className: string; handle: bigint; dwgVersion: number;
  main: Uint8Array; mainBits: number; text: Uint8Array; textBits: number;
  handleStream: Uint8Array; handleBits: number;
  mainSkipBits: number; textSkipBits: number; handleSkipBits: number;
}
