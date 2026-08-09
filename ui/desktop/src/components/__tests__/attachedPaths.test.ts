import { describe, expect, it } from 'vitest';

import { appendAttachmentPaths } from '../attachedPaths';

describe('appendAttachmentPaths', () => {
  it('names an attached image so the model can reach the file', () => {
    const text = appendAttachmentPaths('what is this', [{ path: '/tmp/shot.png' }]);

    expect(text).toBe('what is this /tmp/shot.png');
  });

  it('leaves the text alone for a screenshot, which no file backs', () => {
    const text = appendAttachmentPaths('what is this', [{}]);

    expect(text).toBe('what is this');
  });

  it('waits for an attachment to settle before naming it', () => {
    const text = appendAttachmentPaths('look', [
      { path: '/tmp/loading.png', isLoading: true },
      { path: '/tmp/broken.png', error: 'unreadable' },
      { path: '/tmp/ready.png' },
    ]);

    expect(text).toBe('look /tmp/ready.png');
  });

  it('sends the paths alone when there is nothing to say', () => {
    const text = appendAttachmentPaths('', [{ path: '/a.png' }, { path: '/b.txt' }]);

    expect(text).toBe('/a.png /b.txt');
  });
});
