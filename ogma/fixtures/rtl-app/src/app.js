import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import ar from "./locales/ar.json";

i18n.use(initReactI18next).init({
  lng: "en",
  fallbackLng: "en",
  resources: { en: { translation: en }, ar: { translation: ar } },
});

export function App() {
  const title = t("title");
  const save = t("action.save");
  const cancel = t("action.cancel");
  return [title, save, cancel].join(" ");
}
