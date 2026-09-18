SELECT k.name FROM kind_layout k WHERE k.spec ? 'holds' AND $1 <> ''
