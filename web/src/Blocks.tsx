import type React from "react";
import { inline } from "./markdown";

/**
 * Документ блоками — теми самыми, что разобрал сервер.
 *
 * Прежде интерфейс получал текст простынёй и разбирал его **заново**: где
 * заголовок, где таблица, где код — решал браузер. Разбор при этом уже сделан
 * при записи и лежит в базе. Два разбора одного документа расходятся молча, и
 * этим мы уже обжигались на границе раздела.
 *
 * Здесь разбор один. Браузеру остаётся **строчная** разметка — жирность, код,
 * ссылка: это оформление, а не членение, и своего мнения о документе оно не
 * заводит.
 */
export interface DocBlock {
  ord: number;
  kind: string;
  level: number | null;
  raw: string;
  cells?: { value: string; raw: string }[][];
}

export function Blocks({ blocks }: { blocks: DocBlock[] }): React.JSX.Element {
  return (
    <div className="md blocks">
      {blocks.map((b) => (
        <Block b={b} key={b.ord} />
      ))}
    </div>
  );
}

function Block({ b }: { b: DocBlock }): React.JSX.Element | null {
  switch (b.kind) {
    case "blank":
      return null;

    // Заголовок раздела показывает строка оглавления — она и есть заголовок.
    // Ветка остаётся для тела, в котором заголовок всё-таки встретился: молча
    // выдать его прозой с решётками хуже, чем показать заголовком.
    case "heading": {
      const text = b.raw.replace(/^#+\s*/, "").replace(/\s*#*\s*$/, "");
      const Tag = `h${Math.min((b.level ?? 1) + 1, 6)}` as "h3";
      return <Tag className={`bl-h l${b.level ?? 1}`} dangerouslySetInnerHTML={{ __html: inline(text) }} />;
    }

    case "code":
      // Ограда снимается, содержимое — как есть: код не размечается.
      return <pre className="bl-code">{b.raw.replace(/^[^\n]*\n/, "").replace(/\n?[`~]{3,}\s*$/, "")}</pre>;

    case "table":
      if (!b.cells?.length) return null;
      return (
        <div className="md-table bl-table">
          <table>
            <thead>
              <tr>
                {b.cells[0]!.map((c, i) => (
                  <th key={i} dangerouslySetInnerHTML={{ __html: inline(c.raw.trim()) }} />
                ))}
              </tr>
            </thead>
            <tbody>
              {b.cells.slice(1).map((row, r) => (
                <tr key={r}>
                  {row.map((c, i) => (
                    <td key={i} dangerouslySetInnerHTML={{ __html: inline(c.raw.trim()) }} />
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );

    case "list":
      return (
        <ul className="bl-list">
          {b.raw
            .split("\n")
            .filter((l) => l.trim())
            .map((l, i) => (
              <li
                key={i}
                className={/^\s+/.test(l) ? "deep" : ""}
                dangerouslySetInnerHTML={{ __html: inline(l.replace(/^\s*([-*+]|\d+[.)])\s+/, "")) }}
              />
            ))}
        </ul>
      );

    case "html":
      // Разметка документа не исполняется: она показывается как текст. Иначе
      // набор смог бы вставить в интерфейс что угодно.
      return <pre className="bl-html">{b.raw.trim()}</pre>;

    default: {
      const text = b.raw.trim();
      // Цитата приходит прозой с угловыми скобками в начале строк: разборщик
      // считает её абзацем, и он прав — это абзац. Скобка при этом разметка, а
      // не слово, и оставлять её в тексте значит показывать разметку читателю.
      if (/^>/.test(text) && text.split("\n").every((l) => !l.trim() || l.trimStart().startsWith(">"))) {
        const said = text
          .split("\n")
          .map((l) => l.replace(/^\s*>\s?/, ""))
          .join("\n")
          .trim();
        return <blockquote dangerouslySetInnerHTML={{ __html: inline(said) }} />;
      }
      // Разделительная черта — черта, а не абзац из трёх минусов.
      if (/^([-*_])\1{2,}$/.test(text.replace(/\s+/g, ""))) return <hr className="bl-hr" />;
      return <p dangerouslySetInnerHTML={{ __html: inline(text) }} />;
    }
  }
}
