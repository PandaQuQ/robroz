import { en, type Messages } from './en';
import { zh } from './zh';

export type Locale = 'en' | 'zh';
export type { Messages };

const catalog: Record<Locale, Messages> = { en, zh };

export function t(locale: string | undefined): Messages {
  return locale === 'zh' ? catalog.zh : catalog.en;
}
