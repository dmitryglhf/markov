import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

/**
 * Clipboard screenshots are pixels no file backs, so there is no path to hand
 * the agent. Writing them out gives one, at the cost of a directory that would
 * otherwise grow forever — hence the same keep-the-newest discipline the
 * startup diagnostics use.
 */
const PASTED_IMAGES_TO_KEEP = 20;

const EXTENSIONS: Record<string, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
};

const PREFIX = 'pasted-';

const prune = (dir: string) => {
  const written = fs
    .readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.startsWith(PREFIX))
    .map((entry) => {
      const filePath = path.join(dir, entry.name);
      return { filePath, modifiedMs: fs.statSync(filePath).mtimeMs };
    })
    .sort((a, b) => b.modifiedMs - a.modifiedMs);

  for (const stale of written.slice(PASTED_IMAGES_TO_KEEP)) {
    fs.unlinkSync(stale.filePath);
  }
};

/** Returns the path the image was written to, or null for a format the agent cannot read. */
export function savePastedImage(dir: string, bytes: Uint8Array, mimeType: string): string | null {
  const extension = EXTENSIONS[mimeType];
  if (!extension) {
    return null;
  }

  fs.mkdirSync(dir, { recursive: true });

  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  const filePath = path.join(
    dir,
    `${PREFIX}${stamp}-${crypto.randomBytes(2).toString('hex')}.${extension}`
  );
  fs.writeFileSync(filePath, bytes);
  prune(dir);

  return filePath;
}
