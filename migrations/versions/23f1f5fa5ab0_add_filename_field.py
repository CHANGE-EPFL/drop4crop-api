"""Add filename field

Revision ID: 23f1f5fa5ab0
Revises: ad7ab7b5bfb2
Create Date: 2024-08-02 14:27:20.281290

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa
import sqlmodel


# revision identifiers, used by Alembic.
revision: str = '23f1f5fa5ab0'
down_revision: Union[str, None] = 'ad7ab7b5bfb2'
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    op.add_column('layer', sa.Column('filename', sqlmodel.sql.sqltypes.AutoString(), nullable=True))
    # ### end Alembic commands ###


def downgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    op.drop_column('layer', 'filename')
    # ### end Alembic commands ###
