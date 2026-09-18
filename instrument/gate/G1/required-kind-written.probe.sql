UPDATE kind_layout SET spec = spec || jsonb_build_object('required', true, 'required-why', 'подсадка самотеста') WHERE name = 'postmortem' AND $1 <> ''
