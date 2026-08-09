import { FolderOpen } from 'lucide-react';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../ui/dialog';
import { Button } from '../ui/button';
import { defineMessages, useIntl } from '../../i18n';
import { alsoAt, skillOrigin, type FoundSkill } from './shadows';

const i18n = defineMessages({
  path: {
    id: 'skillDetails.path',
    defaultMessage: 'Path',
  },
  origin: {
    id: 'skillDetails.origin',
    defaultMessage: 'Origin',
  },
  arguments: {
    id: 'skillDetails.arguments',
    defaultMessage: 'Arguments',
  },
  files: {
    id: 'skillDetails.files',
    defaultMessage: 'Files',
  },
  notUsed: {
    id: 'skillDetails.notUsed',
    defaultMessage: 'Not used',
  },
  notUsedValue: {
    id: 'skillDetails.notUsedValue',
    defaultMessage: '{path} answers to this name',
  },
  alsoAt: {
    id: 'skillDetails.alsoAt',
    defaultMessage: 'Also at',
  },
  alsoAtValue: {
    id: 'skillDetails.alsoAtValue',
    defaultMessage: '{path} (not used)',
  },
  readOnlyBuiltin: {
    id: 'skillDetails.readOnlyBuiltin',
    defaultMessage: 'Built into the app, so it cannot be changed here.',
  },
  readOnlyPlugin: {
    id: 'skillDetails.readOnlyPlugin',
    defaultMessage:
      'This came from a plugin, which keeps the original. Editing it here would last only until the plugin updates.',
  },
  openFolder: {
    id: 'skillDetails.openFolder',
    defaultMessage: 'Open folder',
  },
  close: {
    id: 'skillDetails.close',
    defaultMessage: 'Close',
  },
});

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex gap-3 text-sm">
      <span className="w-24 shrink-0 text-text-secondary">{label}</span>
      <span className="min-w-0 break-words font-mono text-xs leading-5">{value}</span>
    </div>
  );
}

export default function SkillDetails({
  skill,
  found,
  onClose,
}: {
  skill: FoundSkill;
  found: FoundSkill[];
  onClose: () => void;
}) {
  const intl = useIntl();
  const origin = skillOrigin(skill.entry);
  const argumentHint = skill.entry.properties?.['argument-hint'];
  const onDisk = origin !== 'builtin';

  return (
    <Dialog open={true} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{skill.entry.name}</DialogTitle>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-2 pt-2 pb-4">
          <p className="text-sm text-text-secondary">{skill.entry.description}</p>

          <Row label={intl.formatMessage(i18n.path)} value={skill.entry.path} />
          <Row label={intl.formatMessage(i18n.origin)} value={origin} />
          {typeof argumentHint === 'string' && argumentHint.length > 0 && (
            <Row label={intl.formatMessage(i18n.arguments)} value={argumentHint} />
          )}
          {(skill.entry.supportingFiles ?? []).map((file) => (
            <Row key={file} label={intl.formatMessage(i18n.files)} value={file} />
          ))}

          {skill.shadowedBy ? (
            <Row
              label={intl.formatMessage(i18n.notUsed)}
              value={intl.formatMessage(i18n.notUsedValue, { path: skill.shadowedBy })}
            />
          ) : (
            alsoAt(skill, found).map((path) => (
              <Row
                key={path}
                label={intl.formatMessage(i18n.alsoAt)}
                value={intl.formatMessage(i18n.alsoAtValue, { path })}
              />
            ))
          )}

          {origin === 'builtin' && (
            <p className="text-sm text-text-secondary">
              {intl.formatMessage(i18n.readOnlyBuiltin)}
            </p>
          )}
          {origin === 'plugin' && (
            <p className="text-sm text-text-secondary">{intl.formatMessage(i18n.readOnlyPlugin)}</p>
          )}
        </div>

        <DialogFooter>
          {onDisk && (
            <Button
              variant="outline"
              className="mr-auto flex items-center gap-2"
              onClick={() => window.electron.openDirectoryInExplorer(skill.entry.path)}
            >
              <FolderOpen className="w-4 h-4" />
              {intl.formatMessage(i18n.openFolder)}
            </Button>
          )}
          <Button onClick={onClose}>{intl.formatMessage(i18n.close)}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
