# Extracted from clai clcd handle(): reverse tool-name map.
to_entries | map({key:.value,value:.key}) | from_entries
