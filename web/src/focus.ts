import { useEffect } from "react";

/**
 * Фокус внутри модального слоя.
 *
 * Слой закрывает собой страницу, но обход по Tab этого не знает: из открытого
 * ящика фокус уходил в раздел под ним, и пришедший с клавиатуры оказывался в
 * том, чего не видит. Ловушка держит обход в пределах слоя и возвращает фокус
 * туда, откуда его забрали.
 *
 * `enabled` вместо условного вызова: хук нельзя звать по условию, а слой
 * бывает закрыт.
 */
export function useFocusTrap(
  слой: { current: HTMLElement | null },
  enabled: boolean,
): void {
  useEffect(() => {
    if (!enabled) return;
    const было = document.activeElement as HTMLElement | null;
    const узел = слой.current;
    if (!узел) return;

    const берущие = (): HTMLElement[] =>
      [...узел.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select, textarea, [tabindex]:not([tabindex="-1"])',
      )].filter((e) => e.offsetParent !== null);

    берущие()[0]?.focus();

    const ключ = (e: KeyboardEvent): void => {
      if (e.key !== "Tab") return;
      const с = берущие();
      if (!с.length) return;
      const первый = с[0]!;
      const последний = с[с.length - 1]!;
      const где = document.activeElement;
      // Край обхода замыкается сам: иначе Tab на последнем уводит за слой.
      if (e.shiftKey && (где === первый || !узел.contains(где))) {
        e.preventDefault();
        последний.focus();
      } else if (!e.shiftKey && где === последний) {
        e.preventDefault();
        первый.focus();
      }
    };
    document.addEventListener("keydown", ключ);
    return () => {
      document.removeEventListener("keydown", ключ);
      // Фокус возвращается на кнопку, которой слой открыли, а не в начало
      // страницы: иначе закрывший ящик начинает обход заново.
      было?.focus?.();
    };
  }, [слой, enabled]);
}
