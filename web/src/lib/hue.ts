const FNV_OFFSET_BASIS = 0x811c9dc5;
const FNV_PRIME = 0x01000193;
const HUE_RANGE = 360;

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const utf8 = new TextEncoder();

/** 32-bit FNV-1a: XOR each byte into the hash, then multiply by the FNV prime modulo 2^32. */
export function fnv1a32(bytes: Uint8Array): number {
  let hash = FNV_OFFSET_BASIS;
  for (const byte of bytes) {
    hash ^= byte;
    // Math.imul keeps the product exact in 32 bits; a plain `*` loses low bits past 2^53.
    hash = Math.imul(hash, FNV_PRIME);
  }
  return hash >>> 0;
}

/**
 * The hue (0..359) an entity is drawn in, derived from its id so every client
 * and the server agree on it without storing it.
 *
 * Algorithm, identical to lein3000.dev's pod hue:
 * 1. Bytes: a canonical UUID (8-4-4-4-12 hex, any case) becomes its 16 raw
 *    bytes in order, the same bytes as Go's `uuid.UUID[:]` or Rust's
 *    `Uuid::as_bytes()`. The whole id is hashed, not a prefix: UUIDv7 ids
 *    created in the same millisecond share their leading timestamp bytes. Any
 *    other string is hashed as UTF-8, so the function is total.
 * 2. Hash: 32-bit FNV-1a, offset basis 0x811c9dc5, prime 0x01000193.
 * 3. Hue: the unsigned hash modulo 360.
 *
 * Only the hue comes from the id. Lightness and chroma are fixed per palette
 * and mode in CSS, so contrast holds whatever hue an id lands on. The shared
 * test vectors live in `hue-vectors.json`.
 */
export function hueFor(id: string): number {
  return fnv1a32(idBytes(id)) % HUE_RANGE;
}

function idBytes(id: string): Uint8Array {
  if (!UUID_PATTERN.test(id)) {
    return utf8.encode(id);
  }
  const hex = id.replaceAll("-", "");
  const bytes = new Uint8Array(16);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}
