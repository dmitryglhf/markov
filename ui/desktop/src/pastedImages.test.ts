import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { savePastedImage } from './pastedImages';

const tempDirs: string[] = [];

function makeTempDir(): string {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pasted-images-test-'));
  tempDirs.push(tempDir);
  return tempDir;
}

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

describe('savePastedImage', () => {
  afterEach(() => {
    while (tempDirs.length > 0) {
      const tempDir = tempDirs.pop();
      if (tempDir) {
        fs.rmSync(tempDir, { recursive: true, force: true });
      }
    }
  });

  it('writes the bytes it was given and returns where they went', () => {
    const dir = makeTempDir();

    const filePath = savePastedImage(dir, PNG, 'image/png');

    expect(filePath).not.toBeNull();
    expect(path.extname(filePath!)).toBe('.png');
    expect(new Uint8Array(fs.readFileSync(filePath!))).toEqual(PNG);
  });

  it('creates the directory on first use', () => {
    const dir = path.join(makeTempDir(), 'not-there-yet');

    expect(savePastedImage(dir, PNG, 'image/png')).not.toBeNull();
  });

  it('refuses a format the agent could not read back', () => {
    const dir = makeTempDir();

    expect(savePastedImage(dir, PNG, 'image/webp')).toBeNull();
    expect(fs.readdirSync(dir)).toEqual([]);
  });

  it('keeps only the twenty newest', () => {
    const dir = makeTempDir();

    const written: string[] = [];
    for (let i = 0; i < 21; i += 1) {
      const filePath = savePastedImage(dir, PNG, 'image/png')!;
      // Same-millisecond writes would make "newest" a coin flip.
      fs.utimesSync(filePath, new Date(), new Date(Date.now() + i * 1000));
      written.push(filePath);
    }

    const remaining = fs.readdirSync(dir).sort();
    expect(remaining).toHaveLength(20);
    expect(remaining).not.toContain(path.basename(written[0]));
    expect(remaining).toContain(path.basename(written[20]));
  });

  it('leaves files it did not write alone', () => {
    const dir = makeTempDir();
    const stranger = path.join(dir, 'notes.txt');
    fs.writeFileSync(stranger, 'keep me');

    for (let i = 0; i < 25; i += 1) {
      savePastedImage(dir, PNG, 'image/png');
    }

    expect(fs.existsSync(stranger)).toBe(true);
  });
});
