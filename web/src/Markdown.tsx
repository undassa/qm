import type React from "react";
import { toBlocks } from "./markdown";

/**
 * Размеченное тело документа. Отрисовка одна на все дроверы: два места, рисующих
 * одно и то же, разошлись бы — и в одном заголовок остался бы решёткой.
 *
 * Вставка через `dangerouslySetInnerHTML` безопасна по построению: сюда приходит
 * только то, что собрал `markdown.ts`, а он сперва экранирует текст целиком и
 * лишь потом ставит теги, которые породил сам.
 */
export function Markdown({ body }: { body: string }): React.JSX.Element {
  return (
    <div className="md">
      {toBlocks(body).map((block, i) => {
        switch (block.kind) {
          case "heading": {
            const Tag = `h${Math.min(block.level + 1, 6)}` as "h3";
            return <Tag key={i} dangerouslySetInnerHTML={{ __html: block.html }} />;
          }
          case "rule":
            return <hr key={i} />;
          case "code":
            return <pre key={i}>{block.text}</pre>;
          case "quote":
            return <blockquote key={i} dangerouslySetInnerHTML={{ __html: block.html }} />;
          case "table":
            return (
              <div key={i} className="md-table">
                <table>
                  <thead>
                    <tr>
                      {block.head.map((cell, c) => (
                        <th key={c} dangerouslySetInnerHTML={{ __html: cell }} />
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {block.rows.map((row, r) => (
                      <tr key={r}>
                        {row.map((cell, c) => (
                          <td key={c} dangerouslySetInnerHTML={{ __html: cell }} />
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          default:
            return <p key={i} dangerouslySetInnerHTML={{ __html: block.html }} />;
        }
      })}
    </div>
  );
}
