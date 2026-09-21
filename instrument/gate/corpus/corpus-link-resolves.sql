SELECT l.entity_kind || ' ' || l.entity_name || ' → ' || l.target_path AS detail FROM project_document_links l WHERE l.project_id = $1 AND l.target_kind = '' AND l.target_path !~ '^[a-z][a-z0-9+.-]*://' AND NOT EXISTS (SELECT 1 FROM project_documents d WHERE d.project_id = l.project_id AND d.entity_kind = split_part(l.target_path, ':', 1) AND (d.entity_name = split_part(l.target_path, ':', 2) OR (d.entity_name = '' AND split_part(l.target_path, ':', 2) = '')))
   -- ЧУЖОЙ ДОМ НЕ СУДИТСЯ, И ЭТО ТА ЖЕ РОЛЬ, ЧТО У СНЯТЫХ ТЕРМИНОВ.
   --
   -- Документ вида, названного `term.foreign-home`, набор не пишет и не
   -- поддерживает: справочник, исследование, инвентарь прошлой реализации.
   -- Его текст сверен при приёме и зафиксирован датой; правя в нём ссылку,
   -- мы делаем эту дату неверной о сегодняшнем тексте и подменяем источник
   -- своей редакцией.
   --
   -- Замер 21.09: единственная находка пункта — `research dsh → register:`.
   -- Документ `register` в наборе не потерян, его НЕ БЫЛО никогда: вид
   -- объявлен `single`, `required: false` и ни разу не заполнялся, а работу
   -- его делает документ вида `index`. То есть ссылка мертва в источнике, и
   -- чинить её значило бы править чужой текст ради зелёного пункта.
   --
   -- Роль уже есть и уже так работает у `retired-term-in-corpus`: одно
   -- понятие, один способ сказать «этот документ не наш».
   AND NOT EXISTS (SELECT 1 FROM scheme($1) h
                    WHERE h.role = 'term.foreign-home' AND h.value = l.entity_kind)
 ORDER BY 1
