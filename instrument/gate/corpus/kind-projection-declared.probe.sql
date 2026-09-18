UPDATE kind_layout SET spec = spec - 'projection' WHERE name = 'decision' AND $1 <> ''
