/**
 * Какой проект означает адрес.
 *
 * Правило одно, а применяется в двух местах: при первой загрузке и при нажатии
 * «назад». Разъехавшись, эти два места дают самую неприятную поломку — «назад»
 * никуда не ведёт, потому что адрес без параметра там ничего не значит, а при
 * загрузке значит «первый по списку».
 */
export interface Choice<T> {
  project: T | null;
  /** Номер из адреса, которому ничего не отвечает; пусто — всё сошлось. */
  unknown: string;
}

export function chooseProject<T extends { projectId: string }>(
  list: readonly T[],
  asked: string | null,
): Choice<T> {
  const first = list[0] ?? null;
  if (!asked) return { project: first, unknown: "" };
  const found = list.find((p) => p.projectId === asked);
  // Не нашли — открываем первый, но об этом говорим: молчаливая подстановка
  // показывает чужие числа под именем, которого человек не выбирал.
  return found ? { project: found, unknown: "" } : { project: first, unknown: asked };
}
