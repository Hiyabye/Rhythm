import type { Extension } from './types';

// What each built-in extension adds to the shell. The backend owns whether an
// extension is on; this only maps its id to UI. Entries without `extension`
// are the core shell and are always shown.
export type SettingsPane = 'extensions' | 'general' | 'boards' | 'cybercampus' | 'personalization' | 'ai' | 'notifications' | 'data';

export const navigation: { id: string; label: string; extension?: string }[] = [
  { id: 'today', label: '오늘' },
  { id: 'inbox', label: '받은 소식', extension: 'sogang_notices' },
  { id: 'search', label: '탐색', extension: 'search' },
  { id: 'bookmark', label: '보관함', extension: 'saved' },
  { id: 'tasks', label: '과제·마감', extension: 'cybercampus' },
  { id: 'calendar', label: '캘린더', extension: 'calendar' },
];

export const settingsPanes: { id: SettingsPane; label: string; icon: string; extension?: string }[] = [
  { id: 'extensions', label: '확장 기능', icon: 'extensions' },
  { id: 'general', label: '일반', icon: 'settings' },
  { id: 'boards', label: '게시판', icon: 'boards', extension: 'sogang_notices' },
  { id: 'cybercampus', label: '사이버캠퍼스', icon: 'tasks', extension: 'cybercampus' },
  { id: 'personalization', label: '개인화', icon: 'person', extension: 'recommendations' },
  { id: 'ai', label: 'AI', icon: 'cpu', extension: 'ai' },
  { id: 'notifications', label: '알림·백그라운드', icon: 'bell', extension: 'proactive' },
  { id: 'data', label: '데이터 및 정보', icon: 'database' },
];

// Extensions that own archive sources, with the settings pane for their boards.
export const sourceExtensions: { id: string; pane: SettingsPane }[] = [
  { id: 'sogang_notices', pane: 'boards' },
  { id: 'cybercampus', pane: 'cybercampus' },
];

export function enabledExtensions(extensions: Extension[] | undefined): Set<string> {
  return new Set((extensions ?? []).filter((extension) => extension.enabled).map((extension) => extension.id));
}

export function contributed<T extends { extension?: string }>(items: T[], enabled: Set<string>): T[] {
  return items.filter((item) => !item.extension || enabled.has(item.extension));
}
