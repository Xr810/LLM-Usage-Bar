import i18n from "@/i18n";
import proEn from "@/i18n/locales/pro.en.json";
import proZh from "@/i18n/locales/pro.zh-CN.json";

/** Pro copy stays in its own files and namespace. It is not merged into zh/en. */
export function ensureProCopy(): void {
  if (!i18n.hasResourceBundle("zh", "pro")) {
    i18n.addResourceBundle("zh", "pro", proZh, true, true);
  }
  if (!i18n.hasResourceBundle("en", "pro")) {
    i18n.addResourceBundle("en", "pro", proEn, true, true);
  }
}
