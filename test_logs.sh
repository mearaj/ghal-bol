#!/bin/bash
echo "Looking for any relay reserve logs:"
grep -i "relay reserve" terminals/2.txt | tail -n 20
echo "Looking for HOP logs:"
grep -i "HOP" terminals/2.txt | tail -n 20
