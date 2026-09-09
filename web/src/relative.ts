/**
 * Относительный адрес от одного документа корпуса к другому.
 *
 * Это самое хрупкое место в ответе на вопрос: реестр прямо называет «ссылку,
 * которая не разрешается» одной из четырёх форм, ответом не считающихся. Писать
 * такой путь руками — гарантированно ошибаться, и ошибка выглядит убедительно:
 * адрес обещает проверяемость и не даёт её.
 */
export function relativeTo(fromPath: string, toPath: string): string {
  const from = fromPath.split("/").slice(0, -1).filter(Boolean);
  const to = toPath.split("/").filter(Boolean);
  let same = 0;
  while (same < from.length && same < to.length - 1 && from[same] === to[same]) same++;
  const up = from.slice(same).map(() => "..");
  const down = to.slice(same);
  return [...up, ...down].join("/");
}

/** Готовая строка ответа: ссылка с подписью, как её пишет корпус. */
export function linkTo(fromPath: string, toPath: string, label: string): string {
  return `[${label}](${relativeTo(fromPath, toPath)})`;
}
