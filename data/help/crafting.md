Crafting and reductions

recipes                 List recovered recipes.
recipes <name>          Show materials, tools and products.
craft <recipe>          Make one recipe batch from carried supplies.
reductions              List recovered dismantling and butchery recipes.
reductions <name>       Show a reduction and its requirements.
reduce <carried item>   Recover the listed products from that exact item.
resources               Count catalogued supplies in your inventory.

Tools must be carried or present in your room. Multiple listed tools are all required. Tools are retained. Ingredients must be carried. Missing materials or tools leave everything unchanged. Remove worn armor or unwield equipment before reducing it. Crafting and reduction require you to be conscious and out of combat.

Example: carry a knife, two pieces of white bread and two slices of pork, then craft ham sandwich. Use eat sandwich to eat the result. At Southfield Metalshop the bullet press turns one steel sheet into six GH3 shells.

The recovered recipes are available to everyone; no recipe-learning step or XP reward exists in the source. This port uses fixed reduction yields. The original reduction's legacy skill rolls have not been transplanted into Savage Soul. Craftsmanship descriptions on recovered gear are separate from condition and wear.

Some development recipes are unfinished. An unavailable recipe explains its missing dependency instead of consuming your supplies. Recipes are maintained in data/savage-soul/crafting.json; help text is maintained in data/help/crafting.md.

Where to start: Rusty in the Royal Oak bar sells sandwich ingredients and a knife. Metalhead in Southfield Metalshop sells metal resources and portable tools; use the fixed machines there. Ron at the charging station sells recovered weapons. To reach Southfield, follow the south road out of Royal Oak into the wasteland and continue south to its southwest corner. Wizards can use goto southfield:southfieldmetalshop.
