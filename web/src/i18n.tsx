import { createContext, useContext, useState, type ReactNode } from "react";

// Interface text in English and Chinese. Strings not listed here stay in English.
export type Locale = "en" | "zh";

const LOCALE_KEY = "codex.locale";

const messages = {
  en: {
    "nav.new": "New",
    "nav.search": "Search",
    "nav.projects": "Projects",
    "nav.artifacts": "Artifacts",
    "nav.scheduled": "Scheduled",
    "nav.customize": "Customize",
    "nav.more": "More",
    "nav.recents": "Recents",
    "account.settings": "Settings",
    "account.usage": "Usage",
    "account.logout": "Log out",
    "account.language": "Language",
    "settings.title": "Settings",
    "settings.account": "Account",
    "settings.name": "Name",
    "settings.email": "Email",
    "settings.language": "Language",
    "settings.languageHint": "The language the app is shown in.",
    "settings.memory": "Memory",
    "settings.workspace": "Workspace",
    "settings.members": "Members",
    "settings.auditLog": "Audit log",
    "settings.close": "Close settings",
    "home.morning": "Good morning",
    "home.afternoon": "Good afternoon",
    "home.evening": "Good evening",
    "home.placeholder": "How can I help you today?",
    "home.recentArtifacts": "Recent artifacts",
    "home.noArtifacts": "No artifacts yet",
    "composer.send": "Send",
    "composer.project": "Project",
    "composer.noProject": "No project",
    "composer.model": "Model (default)",
    "composer.effort": "Effort (default)",
    "signin.signIn": "Sign in",
    "signin.createAccount": "Create account",
    "signin.username": "Username",
    "signin.password": "Password",
    "signin.noAccount": "No account yet?",
    "signin.haveAccount": "Already have an account?",
    "signin.createOne": "Create one",
    "signin.signInLink": "Sign in",
    "signin.please": "Please wait…",
    "search.placeholder": "Search chats, messages, artifacts, projects and project files…",
    "chat.rename": "Rename",
    "chat.archive": "Archive",
    "chat.delete": "Delete",
  },
  zh: {
    "nav.new": "新建",
    "nav.search": "搜索",
    "nav.projects": "项目",
    "nav.artifacts": "Artifacts",
    "nav.scheduled": "定时任务",
    "nav.customize": "自定义",
    "nav.more": "更多",
    "nav.recents": "最近",
    "account.settings": "设置",
    "account.usage": "用量",
    "account.logout": "退出登录",
    "account.language": "语言",
    "settings.title": "设置",
    "settings.account": "账户",
    "settings.name": "名称",
    "settings.email": "邮箱",
    "settings.language": "语言",
    "settings.languageHint": "应用界面显示的语言。",
    "settings.memory": "记忆",
    "settings.workspace": "工作区",
    "settings.members": "成员",
    "settings.auditLog": "审计日志",
    "settings.close": "关闭设置",
    "home.morning": "早上好",
    "home.afternoon": "下午好",
    "home.evening": "晚上好",
    "home.placeholder": "今天需要什么帮助？",
    "home.recentArtifacts": "最近的 Artifacts",
    "home.noArtifacts": "还没有 Artifacts",
    "composer.send": "发送",
    "composer.project": "项目",
    "composer.noProject": "无项目",
    "composer.model": "模型（默认）",
    "composer.effort": "强度（默认）",
    "signin.signIn": "登录",
    "signin.createAccount": "创建账户",
    "signin.username": "用户名",
    "signin.password": "密码",
    "signin.noAccount": "还没有账户？",
    "signin.haveAccount": "已有账户？",
    "signin.createOne": "创建账户",
    "signin.signInLink": "登录",
    "signin.please": "请稍候…",
    "search.placeholder": "搜索聊天、消息、Artifacts、项目和项目文件…",
    "chat.rename": "重命名",
    "chat.archive": "归档",
    "chat.delete": "删除",
  },
} as const;

export type MessageKey = keyof (typeof messages)["en"];

function initialLocale(): Locale {
  try {
    const saved = localStorage.getItem(LOCALE_KEY);
    if (saved === "en" || saved === "zh") return saved;
  } catch {
    // Storage can be unavailable; fall back to the browser language.
  }
  return navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";
}

interface I18nValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: MessageKey) => string;
}

const I18nContext = createContext<I18nValue | null>(null);

export function LocaleProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(initialLocale);
  const setLocale = (next: Locale) => {
    setLocaleState(next);
    try {
      localStorage.setItem(LOCALE_KEY, next);
    } catch {
      // The choice then lasts for this session only.
    }
  };
  const value: I18nValue = {
    locale,
    setLocale,
    t: (key) => messages[locale][key] ?? messages.en[key],
  };
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/** Translated text for the current language, and a way to change it. */
export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n must be used inside LocaleProvider");
  return value;
}
