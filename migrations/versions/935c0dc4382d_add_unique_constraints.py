"""Add unique constraints

Revision ID: 935c0dc4382d
Revises: 00b682c1d768
Create Date: 2024-07-08 15:50:36.591515

"""

from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa
import sqlmodel


# revision identifiers, used by Alembic.
revision: str = "935c0dc4382d"
down_revision: Union[str, None] = "00b682c1d768"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    op.create_unique_constraint(None, "layer", ["layer_name"])
    op.create_unique_constraint(
        None,
        "layer",
        [
            "crop",
            "year",
            "variable",
            "scenario",
            "climate_model",
            "water_model",
        ],
    )
    # ### end Alembic commands ###


def downgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    op.drop_constraint(None, "layer", type_="unique")
    op.drop_constraint(None, "layer", type_="unique")

    # ### end Alembic commands ###
