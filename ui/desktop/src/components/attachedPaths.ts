/** A dropped file or a pasted image, reduced to what naming it in a message needs. */
export interface Attachment {
  path?: string;
  error?: string;
  isLoading?: boolean;
}

/**
 * Appends the paths of settled attachments to the outgoing text.
 *
 * Images travel to the model as pixels, but a path is what lets it open, move
 * or point a tool at the file, so images contribute one too. A screenshot has
 * no path — the clipboard held pixels that no file backs — and contributes
 * nothing.
 */
export function appendAttachmentPaths(text: string, attachments: Attachment[]): string {
  const paths = attachments
    .filter((attachment) => !attachment.error && !attachment.isLoading)
    .map((attachment) => attachment.path)
    .filter((path): path is string => Boolean(path));

  if (paths.length === 0) {
    return text;
  }

  const pathsString = paths.join(' ');
  return text ? `${text} ${pathsString}` : pathsString;
}
